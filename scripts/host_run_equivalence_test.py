#!/usr/bin/env python3
"""Golden-master test for dogfood launch delegation."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import textwrap
import time
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
GOLDEN_PATH = REPO_ROOT / "scripts" / "host_run_equivalence_golden.json"
TARGETS = ("packages", "substrate", "website")
PLATFORM_PACKAGES = "github-proxy consensus github-devloop"
FIXED_TS = "1760000000"


def write_executable(path: Path, content: str) -> None:
    path.write_text(content, encoding="utf-8")
    path.chmod(0o755)


def make_fake_bin(path: Path) -> None:
    write_executable(
        path,
        textwrap.dedent(
            """\
            #!/usr/bin/env python3
            import json
            import os
            import sys
            from pathlib import Path

            out = Path(os.environ["CAPTURE_FILE"])
            keys = [
                "BIN",
                "FKST_GITHUB_REPO",
                "FKST_GITHUB_WRITE",
                "FKST_GITHUB_BOT_LOGIN",
                "FKST_DEVLOOP_UPSTREAM_BRANCH",
                "FKST_DEVLOOP_INTEGRATION_BRANCH",
                "FKST_DEVLOOP_ROLLUP_MERGE",
                "FKST_DEVLOOP_MANAGED_BOT_LOGINS",
                "FKST_GITHUB_PROXY_POLL_LABEL_PREFIX",
                "FKST_RUNTIME_ROOT",
                "FKST_DURABLE_ROOT",
                "FKST_RATE_POOL_ROOT",
                "FKST_DEVLOOP_BOARD_CMD",
            ]
            payload = {
                "cwd": os.getcwd(),
                "argv": sys.argv,
                "env": {key: os.environ[key] for key in keys if key in os.environ},
            }
            out.write_text(json.dumps(payload, sort_keys=True, indent=2) + "\\n", encoding="utf-8")
            """
        ),
    )


def make_fake_date(bin_dir: Path) -> None:
    write_executable(
        bin_dir / "date",
        textwrap.dedent(
            f"""\
            #!/usr/bin/env bash
            if [ "${{1:-}}" = "+%s" ]; then
              printf '%s\\n' "{FIXED_TS}"
              exit 0
            fi
            exec /bin/date "$@"
            """
        ),
    )


def make_fake_tools(bin_dir: Path) -> None:
    write_executable(
        bin_dir / "cargo",
        "#!/usr/bin/env bash\nexit 0\n",
    )
    make_fake_date(bin_dir)


class DogfoodLayout:
    def __init__(self, root: Path, dogfood_script: str) -> None:
        self.root = root
        self.dogfood_root = root / "dogfood"
        self.skill_dir = root / "skill"
        self.bin_dir = root / "bin"
        self.capture = root / "capture.json"
        self.fake_bin = root / "fake-fkst-framework"
        self.substrate_src = root / "substrate-src"
        self.script = self.skill_dir / "dogfood.sh"

        self.skill_dir.mkdir(parents=True)
        self.bin_dir.mkdir()
        self.substrate_src.mkdir()
        (self.substrate_src / "crates").mkdir()
        make_fake_tools(self.bin_dir)
        make_fake_bin(self.fake_bin)
        write_executable(self.script, dogfood_script)
        self._populate_repos()

    def _populate_repos(self) -> None:
        for host in (
            self.dogfood_root / "pkgs-dogfood",
            self.dogfood_root / "substrate-dogfood" / "pkgs",
            self.dogfood_root / "substrate-dogfood" / "sub",
            self.dogfood_root / "website-dogfood" / "pkgs",
            self.dogfood_root / "website-dogfood" / "site",
        ):
            (host / ".git").mkdir(parents=True)

        platform_roots = (
            self.dogfood_root / "pkgs-dogfood",
            self.dogfood_root / "substrate-dogfood" / "pkgs",
            self.dogfood_root / "website-dogfood" / "pkgs",
        )
        for platform in platform_roots:
            for package in PLATFORM_PACKAGES.split():
                (platform / "packages" / package).mkdir(parents=True, exist_ok=True)
            (platform / "scripts").mkdir(parents=True, exist_ok=True)
            shutil.copy2(REPO_ROOT / "scripts" / "run.sh", platform / "scripts" / "run.sh")
            shutil.copy2(REPO_ROOT / "scripts" / "host_entry.sh", platform / "scripts" / "host_entry.sh")
            shutil.copy2(REPO_ROOT / "scripts" / "host_run.sh", platform / "scripts" / "host_run.sh")
            shutil.copy2(REPO_ROOT / "scripts" / "composed_manifest.sh", platform / "scripts" / "composed_manifest.sh")
            shutil.copy2(REPO_ROOT / "scripts" / "bin_bootstrap.sh", platform / "scripts" / "bin_bootstrap.sh")
            shutil.copy2(REPO_ROOT / "scripts" / "bin_cache.py", platform / "scripts" / "bin_cache.py")

        (self.dogfood_root / "website-dogfood" / "site" / ".fkst" / "local-packages" / "site-board").mkdir(
            parents=True,
            exist_ok=True,
        )

    def env(self, target: str) -> dict[str, str]:
        base_path = os.environ.get("PATH", "")
        env = {
            "PATH": f"{self.bin_dir}:{base_path}",
            "DOGFOOD_ROOT": str(self.dogfood_root),
            "DOGFOOD_REPOS": target,
            "DOGFOOD_CONFIG": str(self.root / "missing-config.sh"),
            "SUBSTRATE_SRC": str(self.substrate_src),
            "BIN": str(self.fake_bin),
            "DEVLOOP_PKGS": PLATFORM_PACKAGES,
            "BOT": "test-bot",
            "GH_ORG": "ExampleOrg",
            "UPSTREAM_BRANCH": "dev",
            "INTEGRATION_BRANCH": "integration-test",
            "ROLLUP_MERGE": "auto",
            "MANAGED_BOT_LOGINS": "test-bot,peer-bot",
            "RATE_POOL": str(self.dogfood_root / "rate-pools"),
            "LOGDIR": str(self.dogfood_root),
            "CAPTURE_FILE": str(self.capture),
            "FKST_NO_AUTOBUILD": "1",
            "FKST_GITHUB_WRITE": "0",
            "DUR_PACKAGES": str(self.dogfood_root / "stable-durable-packages"),
            "DUR_SUBSTRATE": str(self.dogfood_root / "stable-durable-substrate"),
            "DUR_WEBSITE": str(self.dogfood_root / "stable-durable-website"),
        }
        env["DOGFOOD_REPOS"] = target
        return env

    def launch(self, target: str) -> dict[str, object]:
        self.capture.unlink(missing_ok=True)
        result = subprocess.run(
            [str(self.script), "start", target],
            cwd=self.root,
            env=self.env(target),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 0:
            raise AssertionError(
                f"dogfood start {target} failed with {result.returncode}\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
            )
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if self.capture.exists():
                return json.loads(self.capture.read_text(encoding="utf-8"))
            time.sleep(0.05)
        raise AssertionError(f"dogfood start {target} did not invoke fake supervise\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}")


def load_golden_launches() -> dict[str, object]:
    return json.loads(GOLDEN_PATH.read_text(encoding="utf-8"))


def normalize(record: dict[str, object], root: Path) -> dict[str, object]:
    root_markers = sorted({str(root), str(root.resolve())}, key=len, reverse=True)

    def norm(value: object) -> object:
        if isinstance(value, str):
            for marker in root_markers:
                value = value.replace(marker, "$ROOT")
            return value
        if isinstance(value, list):
            return [norm(item) for item in value]
        if isinstance(value, dict):
            return {key: norm(item) for key, item in value.items()}
        return value

    return norm(record)  # type: ignore[return-value]


class HostRunEquivalenceTest(unittest.TestCase):
    maxDiff = None

    def test_delegated_dogfood_launch_matches_committed_golden_for_all_targets(self) -> None:
        golden = load_golden_launches()
        self.assertEqual(set(golden), set(TARGETS))
        new_script = (REPO_ROOT / ".claude" / "skills" / "dogfood-github-devloop" / "dogfood.sh").read_text(
            encoding="utf-8"
        )
        with tempfile.TemporaryDirectory() as tmp:
            tmp_root = Path(tmp)
            new_layout = DogfoodLayout(tmp_root / "new", new_script)

            for target in TARGETS:
                with self.subTest(target=target):
                    new_record = normalize(new_layout.launch(target), new_layout.root)
                    self.assertEqual(new_record, golden[target])
                    env = new_record["env"]  # type: ignore[index]
                    self.assertEqual(env["FKST_GITHUB_WRITE"], "1")  # type: ignore[index]
                    self.assertEqual(
                        env["FKST_RUNTIME_ROOT"],  # type: ignore[index]
                        f"$ROOT/dogfood/dogfood-rt-{target}.{FIXED_TS}",
                    )


if __name__ == "__main__":
    unittest.main()
