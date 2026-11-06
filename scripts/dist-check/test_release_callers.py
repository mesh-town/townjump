"""Text-level tests of the package caller jobs in release.yml (stdlib only)."""
import re
import unittest
from pathlib import Path

DIR = Path(__file__).resolve().parent.parent.parent / ".github" / "workflows"
CALLERS = {"deb": "deb.yml", "openwrt-feed": "openwrt-feed.yml", "keenetic-feed": "keenetic-feed.yml"}


def release_text():
    return (DIR / "release.yml").read_text()


def job(text, name):
    body = text.split("\njobs:\n", 1)[1]
    m = re.search(rf"^  {name}:\n(.*?)(?=^  [a-z][a-z-]*:\n|^  #|\Z)", body, re.S | re.M)
    if not m:
        raise AssertionError(f"job {name} not found")
    return m.group(1)


def sub_block(block, key, indent):
    """Keys of the mapping under '<indent>key:' as a dict of name -> value text."""
    m = re.search(rf"^{' ' * indent}{key}:\n((?:{' ' * (indent + 2)}.*\n?)*)", block, re.M)
    if not m:
        return {}
    return dict(re.findall(rf"^{' ' * (indent + 2)}([A-Za-z_]+): *(.*)$", m.group(1), re.M))


def declared(text, section):
    """Declared names under on.workflow_call.<section> with their 'required' flags."""
    wc = re.search(r"^  workflow_call:\n(.*?)(?=^  [a-z_]+:|^\S)", text, re.S | re.M).group(1)
    m = re.search(rf"^    {section}:\n(.*?)(?=^    [a-z]+:|\Z)", wc, re.S | re.M)
    if not m:
        return {}
    out = {}
    for name, body in re.findall(r"^      ([A-Za-z_]+):\n((?:        .*\n?)*)", m.group(1), re.M):
        out[name] = "required: true" in body
    return out


class ReleaseCallers(unittest.TestCase):
    def test_callers_need_publish_job(self):
        for name in CALLERS:
            block = job(release_text(), name)
            self.assertIn("needs: [check-tag, publish]", block, name)
            self.assertIn(f"uses: ./.github/workflows/{CALLERS[name]}", block, name)

    def test_callers_pass_only_declared_inputs(self):
        for name, f in CALLERS.items():
            given = sub_block(job(release_text(), name), "with", 4)
            inputs = declared((DIR / f).read_text(), "inputs")
            for key in given:
                self.assertIn(key, inputs, f"{name}: undeclared input {key}")
            for key, required in inputs.items():
                if required:
                    self.assertIn(key, given, f"{name}: required input {key} missing")

    def test_callers_pass_only_declared_secrets(self):
        for name, f in CALLERS.items():
            given = sub_block(job(release_text(), name), "secrets", 4)
            secrets = declared((DIR / f).read_text(), "secrets")
            for key, value in given.items():
                self.assertIn(key, secrets, f"{name}: undeclared secret {key}")
                self.assertEqual(value, "${{ secrets.%s }}" % key, name)
        self.assertEqual(sub_block(job(release_text(), "keenetic-feed"), "secrets", 4),
                         {"KEENETIC_DEPLOY_KEY": "${{ secrets.KEENETIC_DEPLOY_KEY }}"})

    def test_callers_publish_from_tag_and_new_targets_build_by_script(self):
        text = release_text()
        for name in CALLERS:
            given = sub_block(job(text, name), "with", 4)
            self.assertEqual(given.get("tag"), "${{ github.ref_name }}", name)
            self.assertEqual(given.get("publish"), "true", name)
            self.assertEqual(given.get("pkg_release"), '"1"', name)
            if name != "deb":
                self.assertEqual(given.get("strict"), "true", name)
        build = job(text, "build")
        self.assertIn("      - name: Build (zigbuild)\n        if: matrix.zig && !matrix.mips && !matrix.soft\n", build)
        self.assertIn("      - name: Build (mips)\n        if: matrix.mips\n", build)
        self.assertIn('run: scripts/mips/build.sh "${{ matrix.target }}"', build)
        self.assertRegex(build, r'if \[ "\$\{\{ matrix.mips \}\}" != "true" \]; then\n\s+rustup target add')
        self.assertIn("ziglang==0.16.0", build)
        self.assertIn("--version 0.23.4", build)

    def test_callers_only_read_the_release(self):
        for name in CALLERS:
            perms = sub_block(job(release_text(), name), "permissions", 4)
            self.assertEqual(perms.get("contents"), "read", name)
            self.assertNotIn("contents: write", job(release_text(), name))

    def test_tap_and_winget_untouched(self):
        text = release_text()
        for name, secret in (("tap", "TAP_TOKEN"), ("winget", "WINGET_TOKEN")):
            block = job(text, name)
            self.assertIn(f"{secret}: ${{{{ secrets.{secret} }}}}", block)
        self.assertEqual(len(re.findall(r"secrets\.(?:TAP_TOKEN|WINGET_TOKEN)", text)), 2)
        self.assertNotIn("Package jobs: add new ones here", text)


if __name__ == "__main__":
    unittest.main()
