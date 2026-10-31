import struct
import sys
from pathlib import Path
import tempfile
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from package_windows_portable import build_portable, validate_gui_executable


class PortableTests(unittest.TestCase):
    def executable(self, subsystem=2, machine=0x8664, flags=0x0022):
        data = bytearray(512)
        data[:2] = b'MZ'
        struct.pack_into('<I', data, 0x3c, 0x80)
        data[0x80:0x84] = b'PE\0\0'
        struct.pack_into('<HH', data, 0x84, machine, 1)
        struct.pack_into('<HH', data, 0x94, 0xf0, flags)
        struct.pack_into('<H', data, 0x98, 0x20b)
        struct.pack_into('<H', data, 0x98 + 68, subsystem)
        return bytes(data)

    def fixture(self, root, executable=None):
        binary = root / 'build' / 'mcp-desktop.exe'
        binary.parent.mkdir()
        binary.write_bytes(self.executable() if executable is None else executable)
        (binary.parent / 'app-runtime.dll').write_bytes(b'fixture app-local runtime')
        (binary.parent / 'mcp-hub.exe').write_bytes(b'not the GUI')
        readme, example = root / 'readme.txt', root / 'example.yaml'
        readme.write_text('MCP Hub @VERSION@ WebView2 Runtime 用户目录', encoding='utf-8')
        example.write_text('version: 1\nentries: []\n', encoding='utf-8')
        return binary, readme, example

    def test_zip_preserves_the_gui_dlls_instructions_and_example_without_cli_or_profiles(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            binary, readme, example = self.fixture(root)
            (binary.parent / 'settings.json').write_text('{"not":"to be bundled"}')
            path = build_portable(binary, root / 'out', '0.4.1', readme, example)
            self.assertEqual(path.name, 'MCP.Hub_0.4.1_windows-x64-portable.zip')
            prefix = 'MCP.Hub_0.4.1_windows-x64/'
            with zipfile.ZipFile(path) as archive:
                self.assertEqual(set(archive.namelist()), {prefix + name for name in ['MCP.Hub.exe', 'app-runtime.dll', 'README.txt', 'examples/customer-mcp.yaml']})
                self.assertEqual(archive.read(prefix + 'MCP.Hub.exe'), binary.read_bytes())
                self.assertIn('0.4.1', archive.read(prefix + 'README.txt').decode('utf-8'))
                self.assertIsNone(archive.testzip())

    def test_console_wrong_architecture_dll_and_truncated_images_fail_before_output_creation(self):
        for data in [b'not a PE', self.executable(3), self.executable(machine=0x14c), self.executable(flags=0x2022), self.executable()[:300]]:
            with self.subTest(data_size=len(data)), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                binary, readme, example = self.fixture(root, data)
                with self.assertRaises(ValueError):
                    build_portable(binary, root / 'out', '0.4.1', readme, example)
                self.assertFalse((root / 'out').exists())

    def test_missing_input_and_invalid_version_fail_without_creating_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            binary, readme, example = self.fixture(root)
            with self.assertRaises(ValueError):
                build_portable(binary, root / 'out', '../bad', readme, example)
            with self.assertRaises(FileNotFoundError):
                build_portable(binary, root / 'out', '0.4.1', root / 'missing.txt', example)
            self.assertFalse((root / 'out').exists())

    def test_existing_archive_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            binary, readme, example = self.fixture(root)
            path = build_portable(binary, root / 'out', '0.4.1', readme, example)
            before = path.read_bytes()
            with self.assertRaises(FileExistsError):
                build_portable(binary, root / 'out', '0.4.1', readme, example)
            self.assertEqual(path.read_bytes(), before)


if __name__ == '__main__':
    unittest.main()
