#!/usr/bin/env bash
set -euo pipefail

# Build the MAKI Tagging Quick Guide poster as a 1-page A3 landscape PDF.
# Prerequisites: xelatex (e.g. via MacTeX or texlive-xetex)
#
# Usage:
#   bash doc/quickref/build-tagging-pdf.sh   # from repo root
#   bash build-tagging-pdf.sh                # from doc/quickref/

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

cd "$SCRIPT_DIR"
echo "Building Tagging Quick Guide PDF..."
xelatex -interaction=nonstopmode tagging.tex > /dev/null 2>&1
echo "PDF generated: $SCRIPT_DIR/tagging.pdf"
