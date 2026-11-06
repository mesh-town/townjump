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
        self.assertRegex(build, r'if \[ "\$\{\{ matrix.mips \}\}" != "true" \] && \[ "\$\{\{ matrix.android \}\}" != "true" \]; then\n\s+rustup target add')
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


GATE = "startsWith(github.ref, 'refs/tags/v') && (github.event_name == 'push' || inputs.publish)"
PUBLISHING = ["publish", "tap", "winget", "deb", "openwrt-feed", "keenetic-feed", "dist", "container-image"]


def if_of(block):
    m = re.search(r"^    if: (.+)$", block, re.M)
    return m.group(1).strip() if m else None


class ReleaseWiring(unittest.TestCase):
    def test_release_builds_android_target(self):
        build = job(release_text(), "build")
        self.assertRegex(build, r"- target: aarch64-linux-android\n\s+os: ubuntu-24.04\n\s+zig: false\n\s+android: true")
        self.assertIn("rustup target add --toolchain \"$toolchain\" aarch64-linux-android", build)
        self.assertIn("cargo build --release --locked -p townjump --target aarch64-linux-android", build)
        for var in ("CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER", "CC_aarch64_linux_android", "AR_aarch64_linux_android"):
            self.assertIn(var, build)
        self.assertIn("aarch64-linux-android24-clang", build)
        self.assertIn("llvm-ar", build)
        self.assertIn("/system/bin/linker64", build)

    def test_release_ndk_version_pinned(self):
        build = job(release_text(), "build")
        versions = set(re.findall(r"ndk[;/](\d+\.\d+\.\d+)", build))
        self.assertEqual(versions, {"28.2.13676358"})

    def test_release_dist_caller_passes_deploy_key(self):
        block = job(release_text(), "dist")
        self.assertIn("uses: ./.github/workflows/dist.yml", block)
        self.assertEqual(sub_block(block, "secrets", 4), {"DIST_DEPLOY_KEY": "${{ secrets.DIST_DEPLOY_KEY }}"})
        given = sub_block(block, "with", 4)
        self.assertEqual(given.get("version"), "${{ needs.check-tag.outputs.version }}")
        self.assertEqual(given.get("publish"), "true")
        self.assertIn("needs: [check-tag, publish]", block)

    def test_release_container_permissions(self):
        block = job(release_text(), "container-image")
        self.assertIn("uses: ./.github/workflows/container-image.yml", block)
        self.assertEqual(sub_block(block, "permissions", 4), {"contents": "read", "packages": "write"})
        given = sub_block(block, "with", 4)
        self.assertEqual(given.get("publish"), "true")
        self.assertEqual(given.get("version"), "${{ needs.check-tag.outputs.version }}")

    def test_release_dispatch_default_is_dry(self):
        text = release_text()
        m = re.search(r"^  workflow_dispatch:\n(.*?)(?=^\S)", text, re.S | re.M)
        self.assertIsNotNone(m)
        body = m.group(1)
        self.assertRegex(body, r"publish:\n(?:\s+.*\n)*?\s+type: boolean\n(?:\s+.*\n)*?\s+default: false")
        self.assertNotIn("version:", body)  # the version comes from Cargo.toml, not from an input

    def test_publish_jobs_skip_on_dry_dispatch(self):
        text = release_text()
        for name in PUBLISHING:
            self.assertEqual(if_of(job(text, name)), GATE, name)
        for name in ("check-tag", "build", "windows-installer"):
            self.assertIsNone(if_of(job(text, name)), name)

    def test_expected_assets_include_android(self):
        script = (DIR.parent.parent / "scripts" / "dist" / "publish.sh").read_text()
        listed = re.search(r"local targets=\(\n(.*?)\n  \)", script, re.S).group(1).split()
        matrix = re.findall(r"- target: (\S+)", job(release_text(), "build"))
        self.assertIn("aarch64-linux-android", listed)
        self.assertEqual(sorted(listed), sorted(matrix))


if __name__ == "__main__":
    unittest.main()
