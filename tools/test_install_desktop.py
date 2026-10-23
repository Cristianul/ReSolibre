"""Application identity and launcher compatibility using disposable fake installation roots."""

import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import install_desktop


class LauncherChecks(unittest.TestCase):
    """Keep renamed launchers usable without replacing unrelated entries or following destination links."""

    def test_renamed_launcher_and_owned_previous_install(self):
        """Install the current identity, remove only this checkout's old launcher, and preserve foreign ones."""
        with tempfile.TemporaryDirectory(prefix='resolibre-launcher-') as temporary:
            root = Path(temporary) / 'checkout with spaces'
            binary = root / 'target/debug/resolibre-desktop'
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b'owned executable fixture')
            binary.chmod(0o700)
            asset = root / 'apps/desktop/ui/assets/resolibre.png'
            asset.parent.mkdir(parents=True)
            asset.write_bytes(b'owned icon fixture')
            data = Path(temporary) / 'data'
            previous = data / 'applications/resolibre.desktop'
            previous.parent.mkdir(parents=True)
            old_command = install_desktop.exec_argument(binary.with_name('resolibre-desktop'))
            previous.write_text(f'[Desktop Entry]\nName=ReSolibre\nExec={old_command}\n')
            old_icon = data / 'icons/hicolor/256x256/apps/resolibre.png'
            old_icon.parent.mkdir(parents=True)
            old_icon.write_bytes(asset.read_bytes())
            with patch.object(install_desktop, 'ROOT', root), patch.dict(os.environ, {'XDG_DATA_HOME': str(data), 'PATH': ''}):
                install_desktop.main()
                contents = (data / 'applications/resolibre.desktop').read_text()
                self.assertIn('Name=ReSolibre\n', contents)
                self.assertIn(f'Exec={install_desktop.exec_argument(binary)}\n', contents)
                self.assertIn('StartupWMClass=resolibre\n', contents)
                self.assertFalse(previous.exists())
                self.assertFalse(old_icon.exists())
                previous.write_text('[Desktop Entry]\nName=ReSolibre\nExec=/other/checkout/player\n')
                install_desktop.main()
                self.assertTrue(previous.exists())
                previous.unlink()
                previous.symlink_to(data / 'applications/resolibre.desktop')
                install_desktop.main()
                self.assertTrue(previous.is_symlink())
                current = data / 'applications/resolibre.desktop'
                current.unlink()
                current.symlink_to(asset)
                with self.assertRaises(SystemExit):
                    install_desktop.main()
                self.assertEqual(asset.read_bytes(), b'owned icon fixture')


if __name__ == '__main__':
    unittest.main()
