import json
import tempfile
import unittest
from pathlib import Path

from update_entry import update_entry

REPO = "https://github.com/acme/rpp-fancy"


def packed(version: str) -> dict:
    return {
        "name": "fancy",
        "version": version,
        "rpp": ">=0.5",
        "description": "Fancy things",
        "file": f"fancy-{version}.rpp.tgz",
        "sha256": "a" * 64,
    }


class UpdateEntryTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.registry = Path(self._tmp.name)

    def read(self) -> dict:
        return json.loads((self.registry / "plugins" / "fancy.json").read_text())

    def test_creates_new_plugin_file(self) -> None:
        update_entry(self.registry, REPO, "v1.0.0", packed("1.0.0"))
        self.assertEqual(
            self.read(),
            {
                "name": "fancy",
                "repository": REPO,
                "description": "Fancy things",
                "versions": [
                    {
                        "version": "1.0.0",
                        "url": f"{REPO}/releases/download/v1.0.0/fancy-1.0.0.rpp.tgz",
                        "sha256": "a" * 64,
                        "rpp": ">=0.5",
                    }
                ],
            },
        )

    def test_appends_version_sorted(self) -> None:
        for v in ("1.0.0", "1.10.0", "1.2.0", "1.2.0-rc.1"):
            update_entry(self.registry, REPO, f"v{v}", packed(v))
        versions = [v["version"] for v in self.read()["versions"]]
        self.assertEqual(versions, ["1.0.0", "1.2.0-rc.1", "1.2.0", "1.10.0"])

    def test_rejects_existing_version(self) -> None:
        update_entry(self.registry, REPO, "v1.0.0", packed("1.0.0"))
        with self.assertRaisesRegex(ValueError, "already in the registry"):
            update_entry(self.registry, REPO, "v1.0.0", packed("1.0.0"))

    def test_url_matches_repository(self) -> None:
        update_entry(self.registry, REPO + "/", "v2.0.0", packed("2.0.0"))
        entry = self.read()
        self.assertTrue(entry["versions"][0]["url"].startswith(entry["repository"] + "/releases/download/"))
        with self.assertRaisesRegex(ValueError, "registered to"):
            update_entry(self.registry, "https://github.com/evil/rpp-fancy", "v2.1.0", packed("2.1.0"))


if __name__ == "__main__":
    unittest.main()
