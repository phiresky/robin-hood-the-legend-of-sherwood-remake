import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class PortableLinkerTests(unittest.TestCase):
    def check_linker(self, commands):
        with tempfile.TemporaryDirectory() as directory:
            for command in commands:
                path = Path(directory, command)
                path.write_text(f'#!/bin/sh\nprintf "%s\\n" "{command}" "$@"\n')
                path.chmod(0o755)
            return subprocess.run(
                [str(Path(__file__).with_name("clang-wild-linker.sh")), "object with spaces.o", "-o", "game"],
                env={**os.environ, "PATH": directory}, capture_output=True, text=True, check=True)

    def test_fast_path_preserves_arguments(self):
        result = self.check_linker(["clang", "wild", "cc"])
        self.assertEqual(result.stdout.splitlines(), ["clang", "--ld-path=wild", "object with spaces.o", "-o", "game"])
        self.assertEqual(result.stderr, "")

    def test_portable_path_is_explicit(self):
        result = self.check_linker(["cc"])
        self.assertEqual(result.stdout.splitlines(), ["cc", "object with spaces.o", "-o", "game"])
        self.assertIn("linking with cc", result.stderr)


if __name__ == "__main__":
    unittest.main()
