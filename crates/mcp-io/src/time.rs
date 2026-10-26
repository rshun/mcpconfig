//! 极简时间格式化（无 chrono 依赖）。
//!
//! 只需要两种形态：备份目录 id（`20261026-141012`）与审计时间戳（RFC3339 秒级）。

use std::time::{SystemTime, UNIX_EPOCH};

/// 分解后的 UTC 时间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtcTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl UtcTime {
    pub fn now() -> Self {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self::from_unix(secs)
    }

    pub fn from_unix(secs: u64) -> Self {
        let days = (secs / 86_400) as i64;
        let rem = secs % 86_400;
        let (year, month, day) = civil_from_days(days);
        Self {
            year,
            month,
            day,
            hour: (rem / 3600) as u32,
            minute: ((rem % 3600) / 60) as u32,
            second: (rem % 60) as u32,
        }
    }

    /// `20261026-141012`：适合做目录名，可排序。
    pub fn stamp(&self) -> String {
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// `2026-10-26T14:10:12Z`：审计日志用。
    pub fn rfc3339(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// Howard Hinnant 的 civil_from_days。
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_1970() {
        let t = UtcTime::from_unix(0);
        assert_eq!((t.year, t.month, t.day), (1970, 1, 1));
        assert_eq!(t.stamp(), "19700101-000000");
    }

    #[test]
    fn known_timestamp() {
        // 2026-10-26T14:10:12Z
        let secs = 1_793_023_812u64;
        let t = UtcTime::from_unix(secs);
        assert_eq!(t.year, 2026);
        assert_eq!(t.month, 10);
        assert_eq!(t.rfc3339(), "2026-10-26T14:10:12Z");
    }

    #[test]
    fn leap_day() {
        let t = UtcTime::from_unix(1_709_164_800); // 2024-02-29T00:00:00Z
        assert_eq!((t.year, t.month, t.day), (2024, 2, 29));
    }
}
