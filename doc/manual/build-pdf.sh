#!/usr/bin/env bash
set -euo pipefail

# Build the dam user manual as a single PDF.
# Prerequisites: pandoc, xelatex (e.g. via MacTeX or texlive-xetex), mmdc (mermaid-cli)
#
# Usage:
#   bash doc/manual/build-pdf.sh          # from repo root
#   bash build-pdf.sh                     # from doc/manual/

# Resolve the manual directory (works from repo root or doc/manual/)
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
MANUAL_DIR="$SCRIPT_DIR"
REPO_ROOT="$MANUAL_DIR/../.."
OUTPUT="$MANUAL_DIR/dam-manual.pdf"

# Extract version from Cargo.toml
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO_ROOT/Cargo.toml")
DATE=$(date +%Y-%m-%d)

# Check prerequisites
for cmd in pandoc xelatex mmdc; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "Error: $cmd is not installed." >&2
        case "$cmd" in
            pandoc)  echo "  Install: brew install pandoc" >&2 ;;
            xelatex) echo "  Install: brew install --cask mactex-no-gui" >&2 ;;
            mmdc)    echo "  Install: brew install mermaid-cli" >&2 ;;
        esac
        exit 1
    fi
done

# --- Ordered list of source files ---

FILES=(
    index.md

    # User Guide
    user-guide/01-overview.md
    user-guide/02-setup.md
    user-guide/03-ingest.md
    user-guide/04-organize.md
    user-guide/05-browse-and-search.md
    user-guide/06-web-ui.md
    user-guide/07-maintenance.md

    # Reference Guide
    reference/00-cli-conventions.md
    reference/01-setup-commands.md
    reference/02-ingest-commands.md
    reference/03-organize-commands.md
    reference/04-retrieve-commands.md
    reference/05-maintain-commands.md
    reference/06-search-filters.md
    reference/07-format-templates.md
    reference/08-configuration.md
    reference/09-data-model.md

    # Developer Guide
    developer/01-rest-api.md
    developer/02-module-reference.md
    developer/03-building-and-testing.md
)

# --- Temp directory for intermediate files ---

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT
TMPFILE="$TMPDIR/manual.md"

# --- Concatenate source files ---

section=""
first=true
for f in "${FILES[@]}"; do
    filepath="$MANUAL_DIR/$f"
    if [[ ! -f "$filepath" ]]; then
        echo "Warning: $filepath not found, skipping." >&2
        continue
    fi

    # Detect section changes and insert page breaks
    cur_section="${f%%/*}"
    if [[ "$first" != "true" ]]; then
        if [[ "$cur_section" != "$section" ]]; then
            printf '\n\n\\newpage\n\n' >> "$TMPFILE"
        else
            printf '\n\n\\newpage\n\n' >> "$TMPFILE"
        fi
    fi
    first=false
    section="$cur_section"

    # Append file, rewriting ../screenshots/ to screenshots/ for pandoc
    sed 's|\.\./screenshots/|screenshots/|g' "$filepath" >> "$TMPFILE"
done

# --- Insert page breaks before each "## dam ..." command in reference section ---

sed -i '' 's/^## dam /\\newpage\n\n## dam /g' "$TMPFILE"

# --- Render mermaid diagrams to PNG ---

echo "Rendering mermaid diagrams..."
diagram_count=0

# Extract and render each ```mermaid ... ``` block
# Use awk to find blocks and write them to individual files
awk '
    /^```mermaid/ { capture=1; block++; file=ENVIRON["TMPDIR"] "/mermaid-" block ".mmd"; next }
    /^```/ && capture { capture=0; next }
    capture { print > file }
' TMPDIR="$TMPDIR" "$TMPFILE"

for mmd_file in "$TMPDIR"/mermaid-*.mmd; do
    [[ -f "$mmd_file" ]] || break
    diagram_count=$((diagram_count + 1))
    png_file="${mmd_file%.mmd}.png"
    echo "  Diagram $diagram_count: $(head -1 "$mmd_file" | cut -c1-40)..."
    mmdc -i "$mmd_file" -o "$png_file" -b white -s 2 --quiet 2>/dev/null || {
        echo "  Warning: failed to render $(basename "$mmd_file"), leaving as code block" >&2
        continue
    }
done

# Replace ```mermaid...``` blocks with image references in the markdown
awk -v tmpdir="$TMPDIR" '
    /^```mermaid/ {
        block++
        png = tmpdir "/mermaid-" block ".png"
        if ((getline line < png) > 0) {
            close(png)
            print "![](mermaid-" block ".png){width=100%}\n"
        } else {
            # Render failed — keep original code block
            print
            keep=1
        }
        skip=1
        next
    }
    /^```/ && skip { skip=0; if (keep) { print; keep=0 }; next }
    skip { if (keep) print; next }
    { print }
' "$TMPFILE" > "$TMPDIR/manual-final.md"

echo "Rendered $diagram_count mermaid diagrams."

# --- Create LaTeX header for footer/header ---

cat > "$TMPDIR/header.tex" << 'LATEX'
\usepackage{fancyhdr}
\usepackage{lastpage}
\pagestyle{fancy}
\fancyhf{}
\fancyhead[L]{\small\textit{dam User Manual}}
\fancyhead[R]{\small\textit{v__VERSION__}}
\fancyfoot[C]{\small\thepage}
\fancyfoot[L]{\small dam v__VERSION__}
\fancyfoot[R]{\small __DATE__}
\renewcommand{\headrulewidth}{0.4pt}
\renewcommand{\footrulewidth}{0.4pt}
% Also apply to chapter opening pages (plain style)
\fancypagestyle{plain}{
  \fancyhf{}
  \fancyhead[L]{\small\textit{dam User Manual}}
  \fancyhead[R]{\small\textit{v__VERSION__}}
  \fancyfoot[C]{\small\thepage}
  \fancyfoot[L]{\small dam v__VERSION__}
  \fancyfoot[R]{\small __DATE__}
  \renewcommand{\headrulewidth}{0.4pt}
  \renewcommand{\footrulewidth}{0.4pt}
}
LATEX

# Substitute version and date into the header
sed -i '' "s/__VERSION__/$VERSION/g; s/__DATE__/$DATE/g" "$TMPDIR/header.tex"

# --- Generate PDF ---

echo "Building PDF from ${#FILES[@]} source files..."

pandoc "$TMPDIR/manual-final.md" \
    --from=markdown \
    --pdf-engine=xelatex \
    --toc \
    --toc-depth=2 \
    -V geometry:margin=1in \
    -V documentclass=report \
    -V title="dam User Manual" \
    -V subtitle="v${VERSION} — Digital Asset Manager for Photographers" \
    -V date="$DATE" \
    -V colorlinks=true \
    -V linkcolor=blue \
    -V urlcolor=blue \
    -V monofont="Menlo" \
    --syntax-highlighting=tango \
    --include-in-header="$TMPDIR/header.tex" \
    --resource-path="$MANUAL_DIR:$TMPDIR" \
    -o "$OUTPUT"

echo "PDF generated: $OUTPUT"
