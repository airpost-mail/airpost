"""
The ScenarioRun key, steps, waits and expectations for the Tern server (harness/servers/
backend.py). Mixed into ScenarioRun; test/README.md documents the vocabulary.

A scenario with a top-level `backend:` key gets a fresh server, a user, and a device token
named `engine` that every engine process of the run (restarts included) is given on its
identity line, in the shape that engine reads. Steps act as another device of the same
user (`other-device` unless `device:` names one).
"""
import json

from . import pim
from .servers import backend as backendmod
from .scenario_pim import ScenarioFailure

ENGINE_DEVICE = "engine"
OTHER_DEVICE = "other-device"
# Reads the server's rows for expectations: its own token, so revoking a device's leaves it working.
READER_DEVICE = "harness-reader"


class BackendMixin:
    backend = None

    def _start_backend(self):
        """Starts the server for a `backend:` scenario. `backend: {user: email}` names the
        user; `identity: null` gives the engine a null identity line, as the desktop gives
        C++, and `identity: {key: value}` overrides fields of the one it would get. A missing
        binary skips the scenario."""
        if "backend" not in self.sc:
            return
        from .scenario import ScenarioSkipped   # scenario.py imports this module
        if backendmod.find_backend_binary() is None:
            raise ScenarioSkipped(f"{self.sc['name']} needs the Tern server: build apps/backend or set TERN_BACKEND_BIN")
        opts = self.sc.get("backend") or {}
        self.backend = backendmod.BackendServer(self.work, log_path=self.work / "backend.log")
        self.backend.start()
        self.backend.device(ENGINE_DEVICE, opts.get("user", backendmod.DEFAULT_USER))
        self._note(f"backend {self.backend.binary} on {self.backend.url}, server id {self.backend.server_id}")

    def _attach_backend(self, ms):
        """Gives `ms` the engine device's identity in the shape its binary reads."""
        if not self.backend:
            return
        opts = self.sc.get("backend") or {}
        if "identity" in opts and opts["identity"] is None:
            self._note("engine identity: null (`backend: {identity: null}`)")
            return
        identity, env = backendmod.engine_identity(self.backend, self.backend.devices[ENGINE_DEVICE], ms.binary)
        identity.update(opts.get("identity") or {})
        ms.identity = identity
        ms.extra_env.update(env)
        self._note(f"engine is {backendmod.engine_kind(ms.binary)}: identity {identity.get('type', 'mailspring')}")

    def _stop_backend(self):
        if self.backend:
            self.backend.stop()

    # -- steps ------------------------------------------------------------------------------------

    def _backend(self):
        if not self.backend:
            raise ScenarioFailure("backend steps need a top-level `backend:` key")
        return self.backend

    def _backend_object(self, arg: dict) -> tuple:
        """(objectType, objectId) from `threads:` / `messages:` selectors (one model) or
        `object_type` + `object_id`."""
        if "object_id" in arg:
            return arg.get("object_type", "thread"), arg["object_id"]
        table, ids = self._resolve_model_selector(arg)
        if len(ids) != 1:
            raise ScenarioFailure(f"a backend step needs exactly one model, selector gave {ids}")
        return table.lower(), ids[0]

    def _backend_step(self, op: str, arg: dict):
        b = self._backend()
        if op == "push":
            object_type, object_id = self._backend_object(arg)
            device = arg.get("device", OTHER_DEVICE)
            b.device(device, b.devices[ENGINE_DEVICE].email)
            v = b.push(device, self.ms.account["id"], object_type, object_id, arg["plugin"],
                       pim.resolve_now(arg.get("value", {})), version=arg.get("version"),
                       header_message_id=arg.get("header_message_id"))
            self._note(f"backend: {device} pushed {object_type} {object_id} {arg['plugin']} (v {v})")
            return v
        if op == "get":
            status, headers, _ = b.request("GET", arg.get("url") or arg["path"], device=arg.get("device"))
            if "status" in arg and status != int(arg["status"]):
                raise ScenarioFailure(f"backend.get {arg.get('url') or arg['path']}: HTTP {status}, expected {arg['status']}")
            self._note(f"backend: GET {arg.get('url') or arg['path']} -> {status} {headers.get('Location', '')}")
            return status
        if op == "stop":
            b.stop()
            self._note("backend: stopped")
            return
        if op == "start":
            b.start()
            self._note(f"backend: started on {b.url}")
            return
        if op == "backup":
            path = b.backup(arg.get("name", "backup"))
            self._note(f"backend: backup {path.name}")
            return
        if op == "restore":
            b.restore(arg.get("name", "backup"))
            self._note(f"backend: restored {arg.get('name', 'backup')}; start it with backend.start")
            return
        if op == "revoke":
            device = arg.get("device", ENGINE_DEVICE)
            n = b.revoke(device)
            self._note(f"backend: revoked {n} token(s) of {device}")
            return n
        raise ScenarioFailure(f"unknown backend op {op!r}")

    # -- waits and expectations -----------------------------------------------------------------

    def _backend_wait(self, arg: dict):
        timeout = float(arg.get("timeout", 60))
        for key, check in (("backend_metadata", self.expect_backend_metadata), ("metadata", self.expect_metadata)):
            if key not in arg:
                continue
            last = []

            def ok():
                last[:] = check(arg[key])
                return not last
            try:
                self.ms.wait_for(ok, timeout, f"{key} {json.dumps(arg[key])}", poll=0.25)
            except TimeoutError as e:
                raise TimeoutError(f"{e}\nstill: {'; '.join(last[:4])}")

    def expect_backend_token_secret(self, _arg) -> list:
        """Added to every `backend:` scenario: no device token appears in the engine's log or
        stderr, whichever field of the identity line was refused (docs/backend-plan.md 4.1)."""
        b = self._backend()
        files = [*self.ms.config_dir.glob("mailsync-*.log*"), self.ms.stderr_path]
        out = []
        for f in files:
            text = f.read_text(errors="replace") if f.exists() else ""
            out += [f"{f.name} contains the {d.name} token" for d in b.devices.values() if d.token in text]
        return out

    def expect_backend_metadata(self, arg) -> list:
        """[{threads|messages: selector | object_id, plugin, value: {subset}, v: n | {min, max},
        absent: true}]: the server's row for the model, read through the pages as the engine's
        user with a token of its own. `absent` passes for no row or a tombstone (`{}`, `[]`)."""
        b = self._backend()
        if not b.running:
            return ["backend_metadata: the backend is stopped"]
        b.device(READER_DEVICE, b.devices[ENGINE_DEVICE].email)
        try:
            rows = b.rows(READER_DEVICE, self.ms.account["id"])
        except backendmod.BackendError as e:
            return [f"backend_metadata: {e}"]
        out = []
        for want in arg:
            object_type, object_id = self._backend_object(want)
            row = next((r for r in rows if r["object_id"] == object_id and r["plugin_id"] == want["plugin"]
                        and r["object_type"] == object_type), None)
            label = f"server {object_type} {object_id} {want['plugin']}"
            if want.get("absent"):
                if row and row["value"] not in ({}, [], None):
                    out.append(f"{label} = {json.dumps(row['value'])}, expected none")
                continue
            if row is None:
                out.append(f"{label}: no row (server has {[(r['object_id'], r['plugin_id']) for r in rows][:6]})")
                continue
            for k, v in (want.get("value") or {}).items():
                if not isinstance(row["value"], dict) or row["value"].get(k) != v:
                    out.append(f"{label}.{k} = {(row['value'] or {}).get(k)!r}, expected {v!r}")
            if "v" in want and not _count_ok(row["v"], want["v"]):
                out.append(f"{label} v = {row['v']}, expected {want['v']}")
        return out


def _count_ok(n: int, want) -> bool:
    if isinstance(want, dict):
        return want.get("min", n) <= n <= want.get("max", n)
    return n == int(want)
