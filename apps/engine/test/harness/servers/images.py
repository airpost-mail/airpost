"""Docker images for the server kinds, built on first use.

A case builds the image it needs when its server starts, so a run that selects no case of a
Docker kind never touches Docker. Images are tagged by a hash of their build context and
reused across runs; `docker image inspect` runs once per tag per process, and a lock file
serialises the build across xdist workers and concurrent sessions.
"""
import fcntl
import hashlib
import subprocess
import tempfile
from pathlib import Path

_ready: set = set()


def ensure(tag: str, context: Path) -> str:
    if tag in _ready:
        return tag
    lock = Path(tempfile.gettempdir()) / f"airpost-harness-image-{hashlib.sha1(tag.encode()).hexdigest()[:12]}.lock"
    with open(lock, "w") as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        if subprocess.run(["docker", "image", "inspect", tag], capture_output=True).returncode != 0:
            r = subprocess.run(["docker", "build", "-q", "-t", tag, str(context)], capture_output=True, text=True)
            if r.returncode != 0:
                raise RuntimeError(f"docker build of {tag} failed: {r.stderr[-2000:]}")
    _ready.add(tag)
    return tag
