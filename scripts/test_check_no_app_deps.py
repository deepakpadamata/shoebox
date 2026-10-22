"""Tests for check-no-app-deps.py (run: python3 -m unittest scripts/test_check_no_app_deps.py)."""
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location(
    "guard", pathlib.Path(__file__).with_name("check-no-app-deps.py"))
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


def meta(edges):
    names = {"core": "crates/core", "mem": "crates/mem", "keeper": "apps/keeper", "serde": None}
    packages = []
    for n, rel in names.items():
        path = f"/w/{rel}/Cargo.toml" if rel else f"/registry/{n}/Cargo.toml"
        packages.append({"id": n, "name": n, "manifest_path": path})
    nodes = [{"id": n, "deps": [{"pkg": d} for d in edges.get(n, [])]} for n in names]
    return {"workspace_root": "/w", "workspace_members": ["core", "mem", "keeper"],
            "packages": packages, "resolve": {"nodes": nodes}}


class Guard(unittest.TestCase):
    def test_clean_graph_passes(self):
        self.assertEqual(guard.violations(meta({"keeper": ["core"], "mem": ["core"]})), [])

    def test_direct_dependency_on_app_fails(self):
        self.assertEqual(guard.violations(meta({"core": ["keeper"]})), [("core", "keeper")])

    def test_transitive_dependency_on_app_fails(self):
        self.assertEqual(guard.violations(meta({"mem": ["core"], "core": ["keeper"]})),
                         [("core", "keeper"), ("mem", "keeper")])

    def test_apps_may_depend_on_apps_and_libs(self):
        self.assertEqual(guard.violations(meta({"keeper": ["core", "serde"]})), [])


if __name__ == "__main__":
    unittest.main()
