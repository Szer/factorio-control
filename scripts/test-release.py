import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch
import hashlib
import io
import tempfile

spec = importlib.util.spec_from_file_location("update_factorio", Path(__file__).with_name("update-factorio.py"))
update = importlib.util.module_from_spec(spec)
spec.loader.exec_module(update)


class Response(io.BytesIO):
    url = "https://dl.factorio.com/releases/factorio_headless.tar.xz"


class ReleaseTests(unittest.TestCase):
    def test_version_validation_precedes_network(self):
        with patch.object(update.urllib.request, "urlopen") as request:
            for version in ["latest", "2.1", "2.1.23/../../x", "2.1.23\n", "$(id)"]:
                with self.assertRaises(ValueError):
                    update.release(version)
            request.assert_not_called()

    def test_streamed_release_hash_and_empty_download(self):
        body = b"fixture archive" * 200000
        with patch.object(update.urllib.request, "urlopen", return_value=Response(body)):
            self.assertEqual(update.release("2.1.23"), hashlib.sha256(body).hexdigest())
        with patch.object(update.urllib.request, "urlopen", return_value=Response()):
            with self.assertRaises(ValueError):
                update.release("2.1.23")

    def test_interrupted_download_does_not_publish_release(self):
        with patch.object(update, "release", side_effect=OSError("interrupted")), patch.object(update, "atomic_text") as write, patch("sys.argv", ["update-factorio.py", "2.1.23"]):
            with self.assertRaises(OSError):
                update.main()
            write.assert_not_called()

    def test_atomic_write(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "version"
            update.atomic_text(path, "2.1.23")
            self.assertEqual(path.read_text(), "2.1.23\n")
            self.assertEqual(list(Path(directory).iterdir()), [path])


if __name__ == "__main__":
    unittest.main()
