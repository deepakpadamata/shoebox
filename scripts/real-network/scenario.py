#!/usr/bin/env python3
"""Real-network scenario (M6-01): a "laptop" (the dev server running the app core) and a keeper,
possibly on another network: pair, import a corpus, sync, fetch every original, revoke.

    scenario.py --local                          # both on this machine, relay off (self-test)
    scenario.py --remote user@vps                # the keeper runs on the VPS over ssh, relays on

Writes a JSON report (steps with durations, how the devices reached each other, failures) to
--report and exits non-zero if any step failed. It uses only the HTTP bridge and the keeper CLI,
so it exercises the shipped binaries.
"""
import argparse
import json
import os
import pathlib
import shlex
import signal
import subprocess
import sys
import tempfile
import time
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[2]


def api(port, cmd, args=None):
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/api/{cmd}",
        data=json.dumps(args or {}).encode(),
        headers={"content-type": "application/json"},
    )
    try:
        return json.load(urllib.request.urlopen(req, timeout=120))
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"{cmd}: {json.load(e).get('error', e)}")


class Keeper:
    """The keeper CLI, here or over ssh."""

    def __init__(self, binary, storage, remote=None, relay="default"):
        self.binary, self.storage, self.remote, self.relay = binary, storage, remote, relay
        self.proc = None

    def _cmd(self, *args):
        base = [self.binary, "--storage", self.storage, *args]
        return ["ssh", self.remote, shlex.join(base)] if self.remote else base

    def start(self):
        extra = ["--relay", self.relay]
        if not self.remote:
            extra.append("--no-mdns")
        self.proc = subprocess.Popen(self._cmd("run", "--name", "nightly-keeper", *extra), stderr=subprocess.DEVNULL)
        time.sleep(3)

    def run(self, *args, timeout=120):
        out = subprocess.run(self._cmd(*args), capture_output=True, text=True, timeout=timeout)
        return out.returncode, out.stdout, out.stderr

    def status(self):
        rc, out, _ = self.run("status", "--json")
        return json.loads(out) if rc == 0 else {}

    def stop(self):
        try:
            self.run("stop", timeout=20)
        except Exception:
            pass
        if self.proc:
            try:
                self.proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.proc.kill()


def wait_for(fn, timeout, every=2.0):
    end = time.time() + timeout
    while time.time() < end:
        try:
            v = fn()
            if v:
                return v
        except Exception:
            pass
        time.sleep(every)
    return None


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--local", action="store_true")
    p.add_argument("--remote")
    p.add_argument("--preset", default="small", help="corpus size: small (200 photos) or medium (10,000)")
    p.add_argument("--keeper-bin", default=str(ROOT / "target/release/shoebox-keeper"))
    p.add_argument("--server-bin", default=str(ROOT / "target/release/desktop-dev-server"))
    p.add_argument("--remote-keeper-bin", default="shoebox-keeper")
    p.add_argument("--corpus", help="folder of photos to import (default: generated)")
    p.add_argument("--report", default="real-network-report.json")
    p.add_argument("--timeout", type=int, default=1800)
    a = p.parse_args()
    if not (a.local or a.remote):
        p.error("choose --local or --remote user@host")

    tmp = pathlib.Path(tempfile.mkdtemp(prefix="shoebox-nightly-"))
    report = {"scenario": "pair-import-sync-fetch-revoke", "mode": "local" if a.local else "remote", "photos": 0, "steps": [], "paths": [], "ok": False}
    procs = []

    def step(name, fn):
        t = time.time()
        try:
            detail = fn()
            report["steps"].append({"name": name, "ok": True, "seconds": round(time.time() - t, 1), "detail": detail})
            print(f"ok   {name} ({time.time() - t:.1f}s)")
            return True
        except Exception as e:
            report["steps"].append({"name": name, "ok": False, "seconds": round(time.time() - t, 1), "error": str(e)})
            print(f"FAIL {name}: {e}")
            return False

    port = 14870
    keeper = Keeper(
        a.keeper_bin if a.local else a.remote_keeper_bin,
        str(tmp / "keeper") if a.local else "/tmp/shoebox-nightly-keeper",
        remote=a.remote,
        relay="off" if a.local else "default",
    )
    try:
        if a.remote:
            subprocess.run(["ssh", a.remote, "rm -rf /tmp/shoebox-nightly-keeper"], check=False)
        server_args = [a.server_bin, "--data-dir", str(tmp / "laptop"), "--listen", f"127.0.0.1:{port}"]
        if a.local:
            server_args.append("--local-only")
        procs.append(subprocess.Popen(server_args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        if not wait_for(lambda: api(port, "app_state"), 60, 1):
            raise RuntimeError("the app core did not start")

        corpus = a.corpus
        if not corpus:
            corpus = str(tmp / "corpus")
            if not step("generate corpus", lambda: subprocess.run(
                ["cargo", "run", "-q", "-p", "xtask", "--", "fixtures", "--preset", a.preset, "--out", corpus, "--with-files"],
                cwd=ROOT, check=True, capture_output=True) and "ok"):
                raise RuntimeError("no corpus")
        step("create library", lambda: api(port, "create_library", {"device_name": "nightly-laptop", "role": "phone"}) and "ok")
        step("import", lambda: api(port, "import_paths", {"paths": [corpus if a.corpus else corpus + "/photos"]}))
        total = api(port, "counts")["photos"]
        report["photos"] = total
        keeper.start()

        def pair():
            ticket = api(port, "pair_start", {"role": "keeper"})["ticket"]
            # The laptop side answers "they match" a moment after the code appears.
            subprocess.Popen([sys.executable, "-c", f"import time,urllib.request,json;time.sleep(4);urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:{port}/api/confirm_pairing',data=json.dumps({{'accept':True}}).encode(),headers={{'content-type':'application/json'}}))"])
            rc, out, err = keeper.run("pair", ticket, "--name", "nightly-keeper", "--yes")
            if rc != 0:
                raise RuntimeError(err.strip() or out.strip())
            return "paired"
        if not step("pair", pair):
            raise RuntimeError("pairing failed")

        def sync():
            s = wait_for(lambda: (lambda st: st if st.get("library", {}).get("photos") == total else None)(keeper.status()), a.timeout)
            if not s:
                raise RuntimeError("the keeper never saw every photo")
            return {"photos": s["library"]["photos"]}
        step("sync metadata", sync)

        def fetch():
            s = wait_for(lambda: (lambda st: st if st.get("library", {}).get("held_originals") == total else None)(keeper.status()), a.timeout)
            if not s:
                raise RuntimeError(f"the keeper holds {keeper.status().get('library', {}).get('held_originals')} of {total} originals")
            return {"held": s["library"]["held_originals"], "peers": s.get("peers", [])}
        step("fetch originals", fetch)
        report["paths"] = [{"device": d["name"], "path": d["path"], "latency_ms": d["latency_ms"]} for d in api(port, "devices") if not d["is_this_device"]]

        def revoke():
            dev = next(d for d in api(port, "devices") if d["name"] == "nightly-keeper")
            api(port, "revoke_device", {"id": dev["id"]})
            # A revoked device may be cut off before it can even be told, so the check is what
            # matters: a photo added now never reaches it, and nothing is lost on the laptop.
            extra = ROOT / "fixtures" / "samples" / "screenshot.png"
            api(port, "import_paths", {"paths": [str(extra)]})
            time.sleep(45)
            held = keeper.status().get("library", {}).get("photos")
            if held is not None and held != total:
                raise RuntimeError(f"the revoked keeper still received photos ({held} of {total})")
            if api(port, "counts")["photos"] != total + 1:
                raise RuntimeError("the laptop lost or did not gain photos")
            return {"keeper_membership": keeper.status().get("membership"), "keeper_photos": held}
        step("revoke", revoke)
    finally:
        keeper.stop()
        for pr in procs:
            pr.send_signal(signal.SIGTERM)
        report["ok"] = bool(report["steps"]) and all(s["ok"] for s in report["steps"])
        pathlib.Path(a.report).write_text(json.dumps(report, indent=2))
        print(("PASS" if report["ok"] else "FAIL"), "-> report in", a.report)
    sys.exit(0 if report["ok"] else 1)


if __name__ == "__main__":
    main()
