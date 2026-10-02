import unittest

from release import plan, published_nightlies


class ReleasePlanTest(unittest.TestCase):
    sha = "a" * 40

    def test_schedule_skips_only_successful_matching_nightly(self):
        release = {
            "tag_name": "v0.1.0-nightly.20261001.aaaaaaaaaaaa",
            "draft": False,
            "published_at": "2026-10-01",
            "body": f"Source commit: {self.sha}",
        }
        self.assertEqual(
            plan(
                "0.1.0-alpha.0",
                self.sha,
                "schedule",
                "nightly",
                "refs/heads/main",
                [release],
                "20261002",
            ),
            ("0.1.0-nightly.20261002.aaaaaaaaaaaa", False),
        )
        release["draft"] = True
        self.assertTrue(
            plan(
                "0.1.0",
                self.sha,
                "schedule",
                "nightly",
                "refs/heads/main",
                [release],
                "20261002",
            )[1]
        )

    def test_manual_is_retryable_and_stable_version_is_preserved(self):
        self.assertTrue(
            plan(
                "0.1.0",
                self.sha,
                "workflow_dispatch",
                "nightly",
                "refs/heads/main",
                [],
                "20261001",
            )[1]
        )
        self.assertEqual(
            plan(
                "0.1.0-beta.1",
                self.sha,
                "push",
                "release",
                "refs/tags/v0.1.0-beta.1",
                [],
                "20261001",
            )[0],
            "0.1.0-beta.1",
        )
        with self.assertRaises(ValueError):
            plan(
                "0.1.0", self.sha, "push", "release", "refs/tags/v0.2.0", [], "20261001"
            )

    def test_retention_excludes_stable_beta_and_drafts(self):
        items = [
            {"tag_name": tag, "draft": draft, "published_at": "2026-10-01"}
            for tag, draft in [
                ("v0.1.0", False),
                ("v0.1.0-beta.1", False),
                ("v0.1.0-nightly.20261001.aaaaaaaaaaaa", False),
                ("v0.1.0-nightly.20261002.aaaaaaaaaaaa", True),
            ]
        ]
        self.assertEqual(len(published_nightlies(items)), 1)


if __name__ == "__main__":
    unittest.main()
