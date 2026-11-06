import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "publish_pages.sh"
BRANCH = "main"
CLEAN_ENV = {k: v for k, v in os.environ.items() if not k.startswith("GIT_") and not k.startswith("TOWNJUMP_")}
CLEAN_ENV.update(GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_SYSTEM="/dev/null")


def git(*args, cwd=None, check=True, env=None):
    res = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, env=env or CLEAN_ENV)
    if check and res.returncode:
        raise AssertionError(f"git {args}: {res.stderr}")
    return res.stdout.strip()


class PublishPagesTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.tmp = Path(tmp.name)
        self.remote = str(self.tmp / "remote.git")
        git("init", "-q", "--bare", "-b", BRANCH, self.remote)
        self.n = 0

    def src(self, files):
        self.n += 1
        d = self.tmp / f"src{self.n}"
        for name, text in files.items():
            (d / name).parent.mkdir(parents=True, exist_ok=True)
            (d / name).write_text(text)
        d.mkdir(exist_ok=True)
        return d

    def publish(self, subdir, files, version="1.0.0", extra=(), hook=None, src=None, env_extra=None, remote=None):
        env = dict(CLEAN_ENV)
        env.update(env_extra or {})
        if hook:
            env["TOWNJUMP_PAGES_BEFORE_PUSH"] = hook
        args = [str(SCRIPT), subdir, str(src or self.src(files)), remote or self.remote, BRANCH, version, *extra]
        return subprocess.run(["bash", *args], capture_output=True, text=True, env=env)

    def head(self):
        return git("rev-parse", "--verify", "-q", f"refs/heads/{BRANCH}", cwd=self.remote, check=False)

    def show(self, path):
        return git("show", f"{BRANCH}:{path}", cwd=self.remote)

    def ls(self, tree=""):
        return git("ls-tree", "-r", "--name-only", f"{BRANCH}:{tree}" if tree else BRANCH, cwd=self.remote).split()

    def competitor_hook(self, subdir="openwrt", name="peer.txt"):
        """A before-push hook that publishes another subdirectory once, as a concurrent job would."""
        work = self.tmp / "peer"
        marker = self.tmp / "peer.done"
        script = self.tmp / "peer.sh"
        script.write_text(
            "set -eu\n"
            f"[ ! -e '{marker}' ] || exit 0\n"
            f": > '{marker}'\n"
            f"rm -rf '{work}'\n"
            f"git init -q -b {BRANCH} '{work}'\n"
            f"cd '{work}'\n"
            f"git remote add origin '{self.remote}'\n"
            f"if git fetch -q origin {BRANCH} 2>/dev/null; then git reset -q --hard FETCH_HEAD; fi\n"
            f"mkdir -p {subdir}\n"
            f"echo peer > {subdir}/{name}\n"
            "git add -A\n"
            "git -c user.name=peer -c user.email=peer@example.invalid commit -q -m peer\n"
            f"git push -q origin HEAD:{BRANCH}\n")
        return f"bash '{script}'"

    def test_first_publish_creates_orphan_branch(self):
        res = self.publish("openwrt", {"a/index.txt": "one"})
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertEqual(sorted(self.ls()), [".nojekyll", "openwrt/a/index.txt"])
        self.assertEqual(self.show("openwrt/a/index.txt"), "one")

    def test_keeps_other_subdirs_byte_identical(self):
        self.publish("openwrt", {"f.txt": "ow", "bin/x": "\x00\x01"})
        before = git("rev-parse", f"{BRANCH}:openwrt", cwd=self.remote)
        self.assertEqual(self.publish("keenetic", {"k.txt": "kn"}).returncode, 0)
        self.assertEqual(git("rev-parse", f"{BRANCH}:openwrt", cwd=self.remote), before)
        self.assertEqual(self.show("keenetic/k.txt"), "kn")

    def test_replaces_own_subdir_fully(self):
        self.publish("openwrt", {"old.txt": "o", "keep.txt": "1"})
        self.assertEqual(self.publish("openwrt", {"keep.txt": "2"}, version="1.0.1").returncode, 0)
        self.assertEqual(self.ls("openwrt"), ["keep.txt"])
        self.assertEqual(self.show("openwrt/keep.txt"), "2")

    def test_commit_author_and_tz(self):
        self.publish("openwrt", {"a": "1"})
        self.publish("keenetic", {"b": "2"})
        fmt = git("log", "-1", "--format=%an|%ae|%cn|%ce|%ad|%cd|%P|%s", "--date=format:%z",
                  BRANCH, cwd=self.remote)
        name, mail, cname, cmail, ad, cd, parents, subject = fmt.split("|")
        self.assertEqual((name, cname), ("Mesh Town", "Mesh Town"))
        self.assertEqual((mail, cmail), ("340663027+mesh-town@users.noreply.github.com",) * 2)
        self.assertEqual((ad, cd), ("+0000", "+0000"))
        self.assertEqual(parents, "")
        self.assertEqual(subject, "townjump 1.0.0")
        self.assertEqual(git("rev-list", "--count", BRANCH, cwd=self.remote), "1")

    def test_keeps_nojekyll(self):
        self.publish("openwrt", {"a": "1"})
        self.publish("keenetic", {"b": "2"})
        self.assertIn(".nojekyll", self.ls())

    def test_retries_after_concurrent_push(self):
        self.publish("keenetic", {"k": "1"})
        res = self.publish("openwrt", {"mine.txt": "m"}, hook=self.competitor_hook("apt", "p.txt"))
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("retrying", res.stderr)
        files = self.ls()
        self.assertIn("openwrt/mine.txt", files)
        self.assertIn("apt/p.txt", files)
        self.assertIn("keenetic/k", files)

    def test_retries_after_concurrent_push_on_first_publish(self):
        res = self.publish("openwrt", {"mine.txt": "m"}, hook=self.competitor_hook("keenetic", "p.txt"))
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertEqual(sorted(self.ls()), [".nojekyll", "keenetic/p.txt", "openwrt/mine.txt"])

    def test_rejects_unsafe_subpath(self):
        for bad in (
            "", "/abs", "..", "a/../b", "./a", "a//b", ".git", "a/",
            "../a", "../../a", ".", "a/..", "a/./b", "a/.", ".git/x", "a/.git", "a/.git/x",
        ):
            with self.subTest(sub=bad):
                self.assertEqual(self.publish(bad, {"a": "1"}).returncode, 2)
        self.assertEqual(self.head(), "")

    def test_keep_rejects_unsafe_path(self):
        for bad in ("../a", "/abs", ".git"):
            with self.subTest(keep=bad):
                res = self.publish("-", {"a": "1"}, env_extra={"PAGES_KEEP": bad})
                self.assertEqual(res.returncode, 2)
                self.assertEqual(self.head(), "")

    def test_whole_repo_replaces_everything(self):
        self.assertEqual(self.publish("-", {"old.txt": "o", "d/x": "1"}).returncode, 0)
        res = self.publish("-", {"new.txt": "n"}, version="1.0.1")
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertEqual(sorted(self.ls()), [".nojekyll", "new.txt"])
        self.assertEqual(git("rev-list", "--count", BRANCH, cwd=self.remote), "1")

    def test_keep_paths_survive(self):
        self.publish("linux/debian", {"dists/InRelease": "sig", "pool/a.deb": "a"})
        self.publish("macos", {"x": "1"})
        res = self.publish("-", {"README.md": "r"}, version="2", env_extra={"PAGES_KEEP": "linux/debian"})
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertEqual(sorted(self.ls()), [".nojekyll", "README.md", "linux/debian/dists/InRelease", "linux/debian/pool/a.deb"])

    def test_keep_path_absent_in_old_tree_is_fine(self):
        res = self.publish("-", {"README.md": "r"}, env_extra={"PAGES_KEEP": "linux/debian"})
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertEqual(sorted(self.ls()), [".nojekyll", "README.md"])

    def test_nested_subpath_keeps_siblings(self):
        self.publish("macos/x", {"m.txt": "mac"})
        self.publish("linux/debian", {"pool/old.deb": "old"})
        res = self.publish("linux/debian", {"pool/new.deb": "new"}, version="1.0.1")
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertEqual(self.show("macos/x/m.txt"), "mac")
        self.assertEqual(self.ls("linux/debian"), ["pool/new.deb"])

    def test_empty_remote_ok(self):
        empty = str(self.tmp / "empty.git")
        git("init", "-q", "--bare", empty)
        for sub in ("-", "linux/debian"):
            with self.subTest(sub=sub):
                res = self.publish(sub, {"a": "1"}, remote=empty)
                self.assertEqual(res.returncode, 0, res.stderr)
        self.assertEqual(git("rev-parse", "--verify", f"refs/heads/{BRANCH}", cwd=empty) != "", True)

    def test_lease_miss_retries_once(self):
        self.publish("keenetic", {"k": "1"})
        res = self.publish("-", {"mine.txt": "m"}, hook=self.competitor_hook("peerdir", "p.txt"),
                           env_extra={"PAGES_KEEP": "peerdir"})
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertEqual(res.stderr.count("retrying"), 1)
        self.assertEqual(sorted(self.ls()), [".nojekyll", "mine.txt", "peerdir/p.txt"])

    def test_deploy_key_sets_ssh_command(self):
        fake = self.tmp / "bin"
        fake.mkdir()
        log = self.tmp / "ssh.log"
        (fake / "ssh").write_text(f'#!/bin/sh\necho "$@" >> "{log}"\nexit 1\n')
        (fake / "ssh").chmod(0o755)
        key = self.tmp / "key"
        key.write_text("not a key\n")
        env = {"DEPLOY_KEY_FILE": str(key), "PATH": f"{fake}:{os.environ['PATH']}"}
        res = self.publish("-", {"a": "1"}, env_extra=env, remote="git@github.invalid:mesh-town/x.git")
        self.assertEqual(res.returncode, 1)
        line = log.read_text()
        self.assertIn(f"-i {key}", line)
        self.assertIn("IdentitiesOnly=yes", line)
        self.assertIn("StrictHostKeyChecking=accept-new", line)

    def test_rejects_empty_src(self):
        empty = self.tmp / "empty"
        empty.mkdir()
        self.assertEqual(self.publish("openwrt", {}, src=empty).returncode, 2)
        self.assertEqual(self.publish("openwrt", {}, src=self.tmp / "absent").returncode, 2)
        self.assertEqual(self.head(), "")

    def test_whole_repo_mode_rejects_update(self):
        upd = self.update_script('cp -R "$2/." "$1/"\n')
        self.assertEqual(self.publish("-", {"a": "1"}, extra=upd).returncode, 2)

    def test_keep_needs_whole_repo_mode(self):
        self.assertEqual(self.publish("apt", {"a": "1"}, env_extra={"PAGES_KEEP": "x"}).returncode, 2)

    def test_wrong_argument_count(self):
        res = subprocess.run(["bash", str(SCRIPT), "openwrt"], capture_output=True, text=True, env=CLEAN_ENV)
        self.assertEqual(res.returncode, 2)

    def test_gives_up_after_five_lease_failures(self):
        self.publish("keenetic", {"k": "1"})
        n = self.tmp / "count"
        hook = (f"echo x >> '{n}'; cd '{self.tmp}' && rm -rf peer2 && git clone -q '{self.remote}' peer2 && cd peer2 "
                f"&& echo $(wc -l < '{n}') > bump.txt && git add -A "
                "&& git -c user.name=p -c user.email=p@example.invalid commit -q -m bump "
                f"&& git push -q origin HEAD:{BRANCH}")
        res = self.publish("openwrt", {"mine.txt": "m"}, hook=hook)
        self.assertEqual(res.returncode, 1)
        self.assertEqual(len(n.read_text().split()), 5)
        self.assertNotIn("openwrt/mine.txt", self.ls())
        self.assertIn("keenetic/k", self.ls())

    def test_push_failure_without_branch_change_fails_at_once(self):
        res = subprocess.run(
            ["bash", str(SCRIPT), "openwrt", str(self.src({"a": "1"})), str(self.tmp / "nope.git"), BRANCH, "1"],
            capture_output=True, text=True, env=CLEAN_ENV)
        self.assertEqual(res.returncode, 1)

    def update_script(self, body):
        path = self.tmp / "update.sh"
        path.write_text("set -eu\n" + body)
        return ("--update", f"bash '{path}'")

    def test_update_hook_sees_previous_subdir(self):
        self.publish("openwrt", {"pool/old.deb": "old"})
        seen = self.tmp / "seen"
        upd = self.update_script(f'ls -R "$1" > "{seen}"\ncp -R "$2/." "$1/"\n')
        res = self.publish("openwrt", {"pool/new.deb": "new"}, version="1.0.1", extra=upd)
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("old.deb", seen.read_text())
        self.assertEqual(sorted(self.ls("openwrt")), ["pool/new.deb", "pool/old.deb"])

    def test_update_hook_first_run_sees_empty_dir(self):
        seen = self.tmp / "seen"
        upd = self.update_script(f'ls -A "$1" | wc -l > "{seen}"\ncp -R "$2/." "$1/"\n')
        self.assertEqual(self.publish("apt", {"x": "1"}, extra=upd).returncode, 0)
        self.assertEqual(seen.read_text().strip(), "0")
        self.assertEqual(self.ls("apt"), ["x"])

    def test_update_hook_rerun_after_concurrent_push(self):
        first = self.publish("apt", {"pool/old.deb": "old"}, extra=self.update_script('cp -R "$2/." "$1/"\n'))
        self.assertEqual(first.returncode, 0, first.stderr)
        log = self.tmp / "runs"
        upd = self.update_script(f'ls -A "$1/pool" | tr "\\n" " " >> "{log}"\necho >> "{log}"\ncp -R "$2/." "$1/"\n')
        # the competitor adds a file to apt/ itself, so the second run must see it
        hook = self.competitor_hook("apt/pool", "peer.deb")
        res = self.publish("apt", {"pool/new.deb": "new"}, version="1.0.1", extra=upd, hook=hook)
        self.assertEqual(res.returncode, 0, res.stderr)
        runs = log.read_text().splitlines()
        self.assertEqual(len(runs), 2)
        self.assertNotIn("peer.deb", runs[0])
        self.assertIn("peer.deb", runs[1])
        self.assertEqual(sorted(self.ls("apt")), ["pool/new.deb", "pool/old.deb", "pool/peer.deb"])

    def test_update_hook_failure_aborts(self):
        self.publish("apt", {"keep": "1"}, extra=self.update_script('cp -R "$2/." "$1/"\n'))
        before = self.head()
        res = self.publish("apt", {"x": "1"}, version="2", extra=self.update_script("exit 3\n"))
        self.assertEqual(res.returncode, 1)
        self.assertEqual(self.head(), before)

    def test_before_push_hook_failure_aborts(self):
        res = self.publish("openwrt", {"a": "1"}, hook="exit 4")
        self.assertEqual(res.returncode, 1)
        self.assertEqual(self.head(), "")


if __name__ == "__main__":
    unittest.main()
