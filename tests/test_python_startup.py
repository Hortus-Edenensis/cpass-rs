"""Startup contracts; set CPASS_TEST_FULL_STARTUP=1 to also load real OCR assets."""

import json
import os
import subprocess
import sys
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]


class StartupTests(unittest.TestCase):
    def run_python(self, source, cwd, **environment):
        result = subprocess.run(
            [sys.executable, "-c", source],
            cwd=cwd,
            env={**os.environ, "PYTHONPATH": str(ROOT), **environment},
            capture_output=True,
            text=True,
            timeout=120,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return json.loads(result.stdout)

    def test_config_defaults_and_environment_key_do_not_rewrite_other_providers(self):
        with TemporaryDirectory() as folder:
            result = self.run_python(
                "import config, json; print(json.dumps([str(config.EXPORT_PATH), "
                "str(config.FACE_PATH), config.SEARCHERS]))",
                folder,
                CPASS_OPENAI_API_KEY="",
            )
            self.assertEqual(result, ["export", "faces", []])
            for config_text in ("", "export_path: null\nface_image_path: null\nsearchers: null\n"):
                Path(folder, "config.yml").write_text(config_text, encoding="utf8")
                result = self.run_python(
                    "import config, json; print(json.dumps([str(config.EXPORT_PATH), "
                    "str(config.FACE_PATH), config.SEARCHERS]))",
                    folder,
                    CPASS_OPENAI_API_KEY="",
                )
                self.assertEqual(result, ["export", "faces", []])
            Path(folder, "config.yml").write_text(
                "searchers:\n"
                "  - type: OpenAISearcher\n    api_key: configured-key\n"
                "  - type: JsonApiSearcher\n    api_key: other-key\n",
                encoding="utf8",
            )
            result = self.run_python(
                "import config, json; print(json.dumps([config.SEARCHERS, config.conf['searchers']]))",
                folder,
                CPASS_OPENAI_API_KEY="environment-key",
            )
            self.assertEqual(result[0][0]["api_key"], "environment-key")
            self.assertEqual(result[0][1]["api_key"], "other-key")
            self.assertEqual(result[1][0]["api_key"], "configured-key")
            self.assertIn("configured-key", Path(folder, "config.yml").read_text(encoding="utf8"))

    def test_version_comes_from_application_resources_not_working_directory(self):
        source = """
import json, sys, types
sys.modules['config'] = types.ModuleType('config')
sys.modules['cxapi'] = types.ModuleType('cxapi')
schema = types.ModuleType('cxapi.schema')
schema.AccountInfo = object
sys.modules['cxapi.schema'] = schema
import utils
print(json.dumps(utils.__version__))
"""
        expected = (
            ROOT.joinpath("pyproject.toml")
            .read_text(encoding="utf8")
            .split('version = "')[1]
            .split('"')[0]
        )
        with TemporaryDirectory() as folder:
            Path(folder, "pyproject.toml").write_text(
                'version = "stale-user-copy"\n', encoding="utf8"
            )
            self.assertEqual(self.run_python(source, folder), expected)
            resources = Path(folder, "bundle")
            resources.mkdir()
            resources.joinpath("pyproject.toml").write_text(
                'version = "bundle-version"\n', encoding="utf8"
            )
            frozen_source = source.replace(
                "import utils", f"sys._MEIPASS = {str(resources)!r}\nimport utils"
            )
            self.assertEqual(self.run_python(frozen_source, folder), "bundle-version")

    @unittest.skipUnless(
        os.environ.get("CPASS_TEST_FULL_STARTUP") == "1", "full OCR startup is a package check"
    )
    def test_actual_self_check_is_offline_and_leaves_caller_directory_untouched(self):
        source = f"""
import runpy, socket, sys
def offline(*args, **kwargs):
    raise AssertionError('self-check attempted network access')
socket.socket.connect = offline
socket.getaddrinfo = offline
sys.argv = [{str(ROOT / 'main.py')!r}, '--self-check']
runpy.run_path(sys.argv[0], run_name='__main__')
"""
        with TemporaryDirectory() as folder:
            Path(folder, "config.yml").write_text("invalid: [", encoding="utf8")
            result = self.run_python(source, folder)
            self.assertEqual(result["status"], "ok")
            self.assertEqual(result["question_types"], [0, 1, 2, 3])
            self.assertEqual(result["dependencies"]["ddddocr"], "inference_checked")
            self.assertTrue(result["dependencies"]["cv2"])
            self.assertTrue(result["dependencies"]["onnxruntime"])
            self.assertEqual(sorted(path.name for path in Path(folder).iterdir()), ["config.yml"])
            self.assertEqual(Path(folder, "config.yml").read_text(encoding="utf8"), "invalid: [")


if __name__ == "__main__":
    unittest.main()
