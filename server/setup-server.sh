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
set -euo pipefail
cd "$(dirname "$0")"

chmod +x ./gridmind-referee
echo "gridmind-referee binary is executable."

if command -v pip3 >/dev/null 2>&1; then
    pip3 install -r broker/requirements.txt
    echo "Broker dependency (flask) installed."
else
    echo "WARNING: pip3 not found -- install Python 3 + pip, or skip this" >&2
    echo "if this machine never runs broker/server.py." >&2
fi

echo
echo "Setup complete. See operators-guide.md for how to start the broker/Master/Arenas."
