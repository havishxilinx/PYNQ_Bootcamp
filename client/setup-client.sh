#!/usr/bin/env bash
# Run this ON the PYNQ board itself, from wherever you copied/unzipped the
# `client/` folder to (e.g. after uploading via Jupyter's file browser and
# extracting in a Jupyter Terminal).
#
# pynqp2p needs no `pip install` -- PYNQ_302's own first notebook cell loads
# it directly via sys.path, but ONLY if pynqp2p_pkg/ is at exactly
# /home/root/jupyter_notebooks/pynqp2p_pkg (that path is hardcoded in the
# notebook). This script puts it there. It assumes `requests` is already
# importable on the board (true on every PYNQ image seen so far) -- only
# `getmac` needs vendoring, which the notebook's own bootstrap cell handles.
#
# pynqsim (Genesis, optional) DOES need an actual install, since the
# notebook does a plain `import pynqsim` with no sys.path bootstrap.
set -euo pipefail
cd "$(dirname "$0")"

TARGET_DIR="/home/root/jupyter_notebooks/pynqp2p_pkg"
if [ -d "$TARGET_DIR" ]; then
    echo "Removing existing $TARGET_DIR before reinstalling..."
    rm -rf "$TARGET_DIR"
fi
mkdir -p "$(dirname "$TARGET_DIR")"
cp -r ./pynqp2p_pkg "$TARGET_DIR"
echo "pynqp2p_pkg placed at $TARGET_DIR"

python3 -c "import requests" 2>/dev/null && echo "requests: OK (already present)" \
    || echo "WARNING: 'requests' is not importable on this board's Python -- pynqp2p will fail to import until it's installed (pip3 install requests, or offline via a vendored wheel)." >&2

read -p "Install pynqsim for Genesis support too? [y/N] " install_genesis
if [[ "$install_genesis" =~ ^[Yy]$ ]]; then
    pip3 install --no-index --find-links ./pynqsim_pkg/vendor_wheels -e ./pynqsim_pkg \
        && echo "pynqsim installed." \
        || echo "pynqsim install failed -- Genesis support is optional, the notebooks work fine without it." >&2
fi

echo
echo "Now copy notebooks/*.ipynb into your Jupyter working directory"
echo "(e.g. /home/root/jupyter_notebooks/) and open one."
