"""Text-level tests of the package workflows (stdlib only: the conditions that actionlint cannot judge).

WORKFLOWS lists the files under test.
"""
import re
import unittest
from pathlib import Path

DIR = Path(__file__).resolve().parent.parent.parent / ".github" / "workflows"
WORKFLOWS = {"deb": "deb.yml", "keenetic": "keenetic-feed.yml", "openwrt": "openwrt-feed.yml"}
SIGNED_IF = "inputs.publish && needs.build.outputs['signed'] == 'true'"
PUBLISH_IF = {"deb": SIGNED_IF, "keenetic": "inputs.publish", "openwrt": SIGNED_IF}
CONTAINER = (DIR / "container-image.yml").read_text()  # not in WORKFLOWS: it has no apt/feed publish gate
TIMEOUTS = {"openwrt": ("30", "60", "30")}


def text_of(key):
    return (DIR / WORKFLOWS[key]).read_text()


def job(text, name):
    """Lines of one job: from '  <name>:' to the next two-space key under jobs."""
    body = text.split("\njobs:\n", 1)[1]
    m = re.search(rf"^  {name}:\n(.*?)(?=^  [a-z][a-z-]*:\n|\Z)", body, re.S | re.M)
    if not m:
        raise AssertionError(f"job {name} not found")
    return m.group(1)


def job_if(block):
    m = re.search(r"^    if: (.+)$", block, re.M)
    return m.group(1).strip() if m else None


def trigger_block(text, trigger):
    m = re.search(rf"^  {trigger}:\n(.*?)(?=^  [a-z_]+:|^\S)", text, re.S | re.M)
    if not m:
        raise AssertionError(f"trigger {trigger} not found")
    return m.group(1)


def input_block(text, trigger, name):
    m = re.search(rf"^      {name}:\n(.*?)(?=^      \S|^    \S|\Z)", trigger_block(text, trigger), re.S | re.M)
    if not m:
        raise AssertionError(f"input {name} not found in {trigger}")
    return m.group(1)


class WorkflowTests(unittest.TestCase):
    def each(self):
        return [(key, text_of(key)) for key in WORKFLOWS]

    def test_publish_requires_input_and_signed(self):
        for key in ("deb", "openwrt"):
            self.assertEqual(job_if(job(text_of(key), "publish")), PUBLISH_IF[key])

    def test_keenetic_publish_requires_input(self):
        text = text_of("keenetic")
        self.assertEqual(job_if(job(text, "publish")), PUBLISH_IF["keenetic"])

    def test_build_skipped_without_tag(self):
        for key, text in self.each():
            self.assertEqual(job_if(job(text, "build")), "inputs.tag != ''")
            self.assertIsNone(job_if(job(text, "test")), "the test job must run on push and pull_request")
            self.assertIn("needs: test", job(text, "build"))

    def test_publish_input_default_false(self):
        for key, text in self.each():
            for trig in ("workflow_call", "workflow_dispatch"):
                block = input_block(text, trig, "publish")
                self.assertIn("type: boolean", block)
                self.assertIn("default: false", block)
                self.assertIn("required: true", input_block(text, trig, "tag"))

    def test_only_dist_publish_has_concurrency(self):
        for key, text in self.each():
            if key != "deb":
                self.assertNotRegex(text, r"(?m)^\s*concurrency:")
        self.assertIn("    concurrency:\n      group: townjump-dist-pages\n      cancel-in-progress: false\n",
                      job(text_of("deb"), "publish"))

    def test_no_secret_literals(self):
        for key, text in self.each():
            for ref in re.findall(r"\$\{\{(.*?)\}\}", text):
                if "secrets" in ref:
                    self.assertRegex(ref.strip(), r"^secrets\.[A-Z][A-Z0-9_]*$")
            self.assertNotRegex(text, r"-----BEGIN|ghp_|github_pat_|gho_")
            for line in text.splitlines():
                if re.match(r"\s*[A-Z_]*(KEY|TOKEN|SECRET|PASSWORD)[A-Z_]*:", line):
                    self.assertRegex(line, r"\$\{\{ (secrets|github)\.[a-z_A-Z]+ \}\}|:$", line)

    def test_permissions_and_runner(self):
        for key, text in self.each():
            self.assertRegex(text, r"(?m)^permissions:\n  contents: read\n")
            self.assertEqual(len(re.findall(r"runs-on: ubuntu-24\.04", text)), 3)
            want = TIMEOUTS.get(key, ("30", "30", "30"))
            self.assertEqual(re.findall(r"timeout-minutes: (\d+)", text), list(want))
            self.assertIn("permissions:\n      contents: read", job(text, "publish"))
            self.assertNotIn("contents: write", text)

    def test_each_feed_has_its_own_deploy_key(self):
        keys = {"deb": "DIST_DEPLOY_KEY", "openwrt": "OPENWRT_DEPLOY_KEY", "keenetic": "KEENETIC_DEPLOY_KEY"}
        for key, text in self.each():
            secret = keys[key]
            self.assertRegex(trigger_block(text, "workflow_call"), rf"      {secret}:\n        required: false")
            pub = job(text, "publish")
            self.assertIn(f"{secret}: ${{{{ secrets.{secret} }}}}", pub)
            self.assertIn('[ -z "$%s" ]' % secret, pub)
            self.assertIn("::notice::", pub)
            self.assertIn("exit 0", pub)
            self.assertIn('(umask 077; printf \'%s\\n\' "$' + secret + '" > "$RUNNER_TEMP/deploy-key")', pub)
            for other in set(keys.values()) - {secret}:
                self.assertNotIn(other, text)
            self.assertNotIn("gh" + "-pages", text)
            self.assertNotIn("PAGES_GIT" + "_URL", text)
            self.assertNotIn("x-access-token", text)

    def test_tag_fetched_before_commit_time(self):
        for key, text in self.each():
            build = job(text, "build")
            fetch = build.index('git fetch --depth=1 origin "+refs/tags/$TAG:refs/tags/$TAG"')
            self.assertLess(fetch, build.index('git log -1 --format=%ct "$TAG"'))
            self.assertIn("--pattern 'townjump-*-linux-musl*.tar.gz' --pattern SHA256SUMS", build)

    def test_push_and_pull_request_have_paths(self):
        for key, text in self.each():
            self.assertIn("paths:", trigger_block(text, "push"))
            self.assertIn("paths:", trigger_block(text, "pull_request"))

    def test_deb_declares_optional_signing_secret(self):
        block = trigger_block(text_of("deb"), "workflow_call")
        self.assertRegex(block, r"secrets:\n      APT_GPG_KEY:\n        required: false\n      DIST_DEPLOY_KEY:\n        required: false")

    def test_openwrt_secrets_declared_not_required(self):
        block = trigger_block(text_of("openwrt"), "workflow_call")
        for name in ("OPENWRT_USIGN_KEY", "OPENWRT_APK_KEY", "OPENWRT_DEPLOY_KEY"):
            self.assertRegex(block, rf"secrets:\n(?:      \w+:\n        required: false\n)*      {name}:\n        required: false")

    def test_openwrt_strict_input_and_fakeroot(self):
        text = text_of("openwrt")
        for trig in ("workflow_call", "workflow_dispatch"):
            block = input_block(text, trig, "strict")
            self.assertIn("type: boolean", block)
            self.assertIn("default: false", block)
        self.assertIn("fakeroot scripts/dist/openwrt-build.sh", job(text, "build"))
        self.assertIn("hashFiles('packaging/openwrt/feed/tools.sh')", job(text, "test"))
        self.assertIn('TOWNJUMP_PKG_REQUIRE_TOOLS: "1"', job(text, "test"))

    def test_openwrt_secret_files_are_private(self):
        build = job(text_of("openwrt"), "build")
        self.assertRegex(build, r"umask 077")
        self.assertLess(build.index("umask 077"), build.index("usign.sec"))
        self.assertIn("--ephemeral-keys", build)
        self.assertIn("::notice::", build)

    def test_container_push_only_on_tag_and_publish(self):
        block = job(CONTAINER, "image")
        cond = "inputs.publish && startsWith(github.ref, 'refs/tags/v')"
        self.assertEqual(block.count("push: true"), 1)
        for step in ("Log in to ghcr.io", "Build all platforms and push"):
            m = re.search(rf"- name: {step}\n        if: \$\{{\{{ (.+) \}}\}}", block)
            self.assertTrue(m, step)
            self.assertEqual(m.group(1), cond)
        self.assertEqual(trigger_block(CONTAINER, "workflow_dispatch").count("default: false"), 1)

    def test_container_has_run_check_before_push(self):
        block = job(CONTAINER, "image")
        run = block.index("docker run --rm")
        self.assertIn("--version", block[run:])
        self.assertIn("--platform linux/amd64 --load", block)
        self.assertLess(run, block.index("docker/login-action"))
        self.assertLess(run, block.index("docker/build-push-action"))
        self.assertIn("linux/amd64,linux/arm64,linux/arm/v7", block)

    def test_container_permissions(self):
        block = job(CONTAINER, "image")
        self.assertRegex(block, r"permissions:\n      contents: read\n      packages: write\n")
        self.assertNotIn("secrets.", CONTAINER)


class DistWorkflowTests(unittest.TestCase):
    """dist.yml: direct files in the dist repository. Kept out of WORKFLOWS: it has one job, not test/build/publish."""

    text = (DIR / "dist.yml").read_text()

    def test_dist_shares_concurrency_group_with_deb(self):
        block = job(self.text, "tree")
        want = "    concurrency:\n      group: townjump-dist-pages\n      cancel-in-progress: false\n"
        self.assertIn(want, block)
        self.assertIn(want, job(text_of("deb"), "publish"))
        self.assertIn("permissions:\n      contents: read", block)

    def test_dist_publish_keeps_debian_dir(self):
        self.assertRegex(self.text, r"PAGES_KEEP=linux/debian")
        self.assertIn('publish_pages.sh - tree "$DIST_GIT_URL" "$PAGES_BRANCH" "$VERSION"', self.text)
        self.assertIn("DEPLOY_KEY_FILE=", self.text)

    def test_dist_skips_without_key(self):
        pub = self.text.split("- name: Publish\n", 1)[1]
        self.assertIn("        if: inputs.publish\n", pub)
        self.assertIn('[ -z "$DIST_DEPLOY_KEY" ]', pub)
        self.assertIn("::notice::", pub)
        self.assertIn("exit 0", pub)
        self.assertRegex(trigger_block(self.text, "workflow_call"), r"      DIST_DEPLOY_KEY:\n        required: false")
        self.assertNotRegex(self.text, r"-----BEGIN|ghp_|github_pat_|gho_")

    def test_dist_publish_default_false_on_dispatch(self):
        for trig in ("workflow_call", "workflow_dispatch"):
            block = input_block(self.text, trig, "publish")
            self.assertIn("type: boolean", block)
            self.assertIn("default: false", block)
            self.assertIn("required: true", input_block(self.text, trig, "version"))


if __name__ == "__main__":
    unittest.main()
