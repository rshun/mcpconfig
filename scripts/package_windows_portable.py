"""Package the Windows GUI built by the installer job; uses only the standard library."""
import argparse
from pathlib import Path
import re
import struct
import zipfile


def validate_gui_executable(data):
    """Require an AMD64 PE32+ GUI executable, rejecting a renamed console CLI or DLL.

    Header offsets: https://learn.microsoft.com/en-us/windows/win32/debug/pe-format
    """
    if len(data) < 64 or data[:2] != b'MZ':
        raise ValueError('Portable GUI requires a Windows PE executable')
    pe = struct.unpack_from('<I', data, 0x3c)[0]
    if pe < 64 or pe + 24 > len(data) or data[pe:pe + 4] != b'PE\0\0':
        raise ValueError('Invalid PE header')
    machine, sections = struct.unpack_from('<HH', data, pe + 4)
    size, flags = struct.unpack_from('<HH', data, pe + 20)
    optional = pe + 24
    if size < 112 or optional + size + sections * 40 > len(data) or not sections:
        raise ValueError('Truncated PE optional header or section table')
    magic = struct.unpack_from('<H', data, optional)[0]
    subsystem = struct.unpack_from('<H', data, optional + 68)[0]
    if machine != 0x8664 or magic != 0x20b:
        raise ValueError('Portable GUI must be Windows x64 (PE32+)')
    if not flags & 0x0002 or flags & 0x2000 or subsystem != 2:
        raise ValueError('Portable package must contain the GUI executable, not CLI or DLL')


def build_portable(binary, output, version, readme, example):
    if not re.fullmatch(r'\d+\.\d+\.\d+', version):
        raise ValueError('Version must be a stable semantic version')
    binary, output = Path(binary), Path(output)
    content = binary.read_bytes()
    validate_gui_executable(content)
    # Build output may contain app-local runtime DLLs. Keep them with the executable.
    payload = {'MCP.Hub.exe': content}
    for dll in sorted(binary.parent.glob('*.dll')):
        if not dll.is_file() or dll.is_symlink():
            raise ValueError('App-local DLL must be a regular file')
        payload[dll.name] = dll.read_bytes()
    payload['README.txt'] = Path(readme).read_text(encoding='utf-8-sig').replace('@VERSION@', version).encode('utf-8')
    payload['examples/customer-mcp.yaml'] = Path(example).read_bytes()
    if len({name.casefold() for name in payload}) != len(payload):
        raise ValueError('Duplicate Windows file names')
    prefix = f'MCP.Hub_{version}_windows-x64'
    output.mkdir(parents=True, exist_ok=True)
    archive_path = output / f'{prefix}-portable.zip'
    # Never overwrite a prior package. A failed CI job can be rerun on a clean runner.
    with zipfile.ZipFile(archive_path, 'x', zipfile.ZIP_DEFLATED) as archive:
        for name, data in payload.items():
            archive.writestr(f'{prefix}/{name}', data)
    with zipfile.ZipFile(archive_path) as archive:
        expected = {f'{prefix}/{name}' for name in payload}
        if set(archive.namelist()) != expected or archive.testzip() is not None:
            raise ValueError('Portable ZIP contents or CRC validation failed')
        for name, data in payload.items():
            if archive.read(f'{prefix}/{name}') != data:
                raise ValueError('Portable ZIP differs from the installer build')
    return archive_path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--readme', default='docs/WINDOWS-PORTABLE.txt')
    parser.add_argument('--example', default='examples/customer-mcp.yaml')
    args = parser.parse_args()
    print(build_portable(args.binary, args.output, args.version, args.readme, args.example))


if __name__ == '__main__':
    main()
