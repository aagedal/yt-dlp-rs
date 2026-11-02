import unittest

from semver_check import crate_version, policy


class SemverCheckTests(unittest.TestCase):
    def test_breaking_changes_fail_only_after_one_point_zero(self):
        self.assertEqual(policy('0.7.0-beta.2', 'v0.7.0-beta.2'), 'report')
        self.assertEqual(policy('0.8.0', 'v0.7.0-beta.2'), 'report')
        self.assertEqual(policy('1.0.0', 'v0.9.0'), 'skip')
        self.assertEqual(policy('1.0.0', 'v1.0.0'), 'enforce')
        self.assertEqual(policy('1.2.0-rc.1', 'v1.1.3'), 'enforce')
        self.assertEqual(policy('2.0.0-beta.1', 'v1.4.0'), 'skip')

    def test_crate_version_is_the_package_version(self):
        manifest = '[package]\nname = "yt-dlp-rs"\nversion = "0.7.0-beta.2"\n' \
            '[dependencies]\nfoo = { version = "1" }\n'
        self.assertEqual(crate_version(manifest), '0.7.0-beta.2')


if __name__ == '__main__':
    unittest.main()
