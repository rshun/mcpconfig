//! 调用操作系统自带的目录对话框，不通过 shell 拼接用户路径。
use std::{path::PathBuf, process::Command};

pub fn choose(initial: Option<String>) -> Result<Option<String>, String> {
    let initial = initial.filter(|path| {
        std::path::Path::new(path).is_absolute() && std::path::Path::new(path).is_dir()
    });
    let mut command = command_with_initial(initial.as_deref());
    let output = command
        .output()
        .map_err(|_| "无法打开系统目录选择器，请手动输入绝对目录".to_string())?;
    #[cfg(target_os = "macos")]
    if !output.status.success() && String::from_utf8_lossy(&output.stderr).contains("(-128)") {
        return Ok(None);
    }
    if !output.status.success() {
        return Err("目录选择器未完成，请重试或手动输入绝对目录".into());
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| "目录路径编码无效".to_string())?;
    #[cfg(windows)]
    let selected: Option<String> = serde_json::from_str(text.trim_start_matches('\u{feff}').trim())
        .map_err(|_| "目录选择结果无效".to_string())?;
    #[cfg(not(windows))]
    let selected = if text.trim().is_empty() {
        None
    } else {
        Some(text.trim_end_matches(['\r', '\n']).to_string())
    };
    match selected {
        None => Ok(None),
        Some(selected) => {
            let path = PathBuf::from(selected);
            if !path.is_absolute() || !path.is_dir() {
                return Err("请选择有效的绝对目录".into());
            }
            Ok(Some(
                path.to_str().ok_or("目录路径编码不受支持")?.to_string(),
            ))
        }
    }
}

fn command_with_initial(initial: Option<&str>) -> Command {
    let mut command = picker_command();
    if let Some(initial) = initial {
        command.env("MCP_HUB_PICKER_INITIAL", initial);
    }
    command
}

#[cfg(windows)]
fn picker_command() -> Command {
    use std::os::windows::process::CommandExt;
    let mut command = Command::new("powershell.exe");
    command.creation_flags(0x08000000);
    command.args([
        "-NoProfile",
        "-STA",
        "-Command",
        r#"
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
Add-Type -AssemblyName System.Windows.Forms
$picker = New-Object System.Windows.Forms.FolderBrowserDialog
$picker.Description = '选择 MCP 项目目录'
$picker.ShowNewFolderButton = $true
if ($env:MCP_HUB_PICKER_INITIAL) { $picker.SelectedPath = $env:MCP_HUB_PICKER_INITIAL }
try {
  if ($picker.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
    ConvertTo-Json -InputObject $picker.SelectedPath -Compress
  } else { 'null' }
} finally { $picker.Dispose() }
"#,
    ]);
    command
}

#[cfg(target_os = "macos")]
fn picker_command() -> Command {
    let mut command = Command::new("/usr/bin/osascript");
    command.args([
        "-e",
        "POSIX path of (choose folder with prompt \"选择 MCP 项目目录\")",
    ]);
    command
}

#[cfg(not(any(windows, target_os = "macos")))]
fn picker_command() -> Command {
    // Linux GUI 不在发布范围，始终返回错误，前端保留手动输入。
    Command::new("/nonexistent/mcp-hub-directory-dialog")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn picker_uses_fixed_script_and_never_embeds_user_input() {
        let untrusted = "C:/project/中文'; throw 'your_untrusted_path_here'; #";
        let command = command_with_initial(Some(untrusted));
        let args = command
            .get_args()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(!args.contains("your_untrusted_path_here"));
        assert!(command
            .get_envs()
            .any(|(key, value)| key == "MCP_HUB_PICKER_INITIAL"
                && value == Some(std::ffi::OsStr::new(untrusted))));
        #[cfg(windows)]
        {
            assert!(args.contains("-STA"));
            assert!(args.contains("$env:MCP_HUB_PICKER_INITIAL"));
        }
    }
}
