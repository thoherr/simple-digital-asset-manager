#!/usr/bin/env bash
set -euo pipefail

# Build the MAKI Search Filter Reference as a 2-page A4 PDF.
# Prerequisites: pandoc, xelatex (e.g. via MacTeX or texlive-xetex)
#
# Usage:
#   bash doc/manual/build-search-filters-pdf.sh    # from repo root
#   bash build-search-filters-pdf.sh               # from doc/manual/

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
MANUAL_DIR="$SCRIPT_DIR"
REPO_ROOT="$MANUAL_DIR/../.."
OUTPUT="$MANUAL_DIR/search-filters.pdf"

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO_ROOT/Cargo.toml")
DATE=$(date +%Y-%m-%d)

for cmd in pandoc xelatex; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "Error: $cmd is not installed." >&2
        exit 1
    fi
done

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

cat > "$TMPDIR/header.tex" << 'LATEX'
\usepackage{fancyhdr}
\usepackage{graphicx}
\usepackage{xcolor}
\usepackage{enumitem}

% Compact body text
\usepackage{titlesec}
\titleformat{\section}{\normalsize\bfseries}{}{0em}{}
\titleformat{\subsection}{\small\bfseries}{}{0em}{}
\titlespacing{\section}{0pt}{6pt}{2pt}
\titlespacing{\subsection}{0pt}{4pt}{1pt}
\setlength{\parskip}{2pt}

% Tighter table rows (not the manual's 1.4 — we need to fit on one page)
\renewcommand{\arraystretch}{1.1}

% Brand colors
\definecolor{maki-salmon}{HTML}{e8634a}
\definecolor{maki-nori}{HTML}{1a2332}
\definecolor{maki-stone}{HTML}{556677}

% Header/footer
\pagestyle{fancy}
\fancyhf{}
\fancyhead[L]{\raisebox{-2pt}{\includegraphics[height=10pt]{__MANUAL_DIR__/maki-icon-header.png}}\;\small\textit{\textcolor{maki-stone}{MAKI Search Filter Reference}}}
\fancyhead[R]{\small\textit{\textcolor{maki-stone}{v__VERSION__}}}
\fancyfoot[C]{\small\thepage}
\fancyfoot[L]{\small\textcolor{maki-stone}{MAKI v__VERSION__}}
\fancyfoot[R]{\small\textcolor{maki-stone}{__DATE__}}
\renewcommand{\headrulewidth}{0.4pt}
\renewcommand{\footrulewidth}{0.4pt}

% No title page — start directly with content
\renewcommand{\maketitle}{}
LATEX

MANUAL_DIR_ESCAPED=$(echo "$MANUAL_DIR" | sed 's/\//\\\//g')
sed -i '' "s/__VERSION__/$VERSION/g; s/__DATE__/$DATE/g; s/__MANUAL_DIR__/$MANUAL_DIR_ESCAPED/g" "$TMPDIR/header.tex"

echo "Building Search Filter Reference PDF..."

pandoc "$MANUAL_DIR/search-filters.md" \
    --from=markdown \
    --pdf-engine=xelatex \
    -V geometry:margin=0.7in \
    -V geometry:top=0.8in \
    -V geometry:bottom=0.7in \
    -V documentclass=extarticle \
    -V fontsize=8pt \
    -V title=" " \
    -V date=" " \
    -V colorlinks=true \
    -V linkcolor=blue \
    -V urlcolor=blue \
    -V monofont="Menlo" \
    --syntax-highlighting=tango \
    --include-in-header="$TMPDIR/header.tex" \
    --resource-path="$MANUAL_DIR" \
    -o "$OUTPUT"

echo "PDF generated: $OUTPUT"
