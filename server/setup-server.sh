#!/usr/bin/env bash
# One-time setup for a GridMind server-role machine (broker and/or Master
# and/or Arena -- this same package works for any/all of those roles).
#
# No build step: gridmind-referee is a prebuilt binary. The only install
# step is the broker's one Python dependency (Flask) -- not vendored here
# since a compiled-wheel match for an unknown target Python version can't
# be guaranteed offline; this is a standard, low-risk PyPI package.
# Skip this script entirely on a machine that only runs the broker's
# server.py is not needed there -- and skip the pip install if this
# machine will only run `gridmind-referee arena`/`master` (no broker).
#
# Genesis (genesis/) is NOT set up by this script -- it's optional per
# arena machine, and its dependencies (torch, genesis-world) need a
# GPU-backend decision (cpu/amdgpu/cuda) this script can't make for you.
# See genesis/QUICKSTART.md.
set -euo pipefail
cd "$(dirname "$0")"

chmod +x ./gridmind-referee
echo "gridmind-referee binary is executable."

if command -v python3 >/dev/null 2>&1; then
    # A venv, not a system-wide pip install: modern distros (PEP 668,
    # "externally-managed-environment") refuse `pip install` onto system
    # Python outright, and installing system-wide would be the wrong call
    # even where it's still allowed.
    python3 -m venv broker/venv
    broker/venv/bin/pip install -r broker/requirements.txt
    echo "Broker dependency (flask) installed into broker/venv."
    echo "Start the broker with: cd broker && venv/bin/python server.py ..."
else
    echo "WARNING: python3 not found -- install Python 3, or skip this" >&2
    echo "if this machine never runs broker/server.py." >&2
fi

echo
echo "Setup complete. See operators-guide.md for how to start the broker/Master/Arenas."
echo "If this arena machine also runs Genesis, see genesis/QUICKSTART.md to set it up."
