#!/usr/bin/env bash
# Rebuilds ui/vendor/pdf.js, pdf.worker.js and pdf-lib.js (the PDF viewer's libraries) from npm, checked against npm's
# published sha512. pdf.js ships as ES modules; Valerie injects plain scripts into PDF tabs, so the module's exports
# become globalThis.pdfjsLib (the worker already sets globalThis.pdfjsWorker, which runs it on the page's thread).
#   scripts/vendor-pdf.sh            (versions below; bump them and the hashes together)
set -euo pipefail
PDFJS=6.4.299
PDFJS_SHA="sha512-AVl138zALtfaAPvADulE0PZThbYzCBS79nL4pOSL/6Sm/4AH5A21BD9VHt97OlCuzJuCpmeZtAtkinisF4Vb1g=="
PDFLIB=1.17.1
PDFLIB_SHA="sha512-V/mpyJAoTsN4cnP31vc0wfNA1+p20evqqnap0KLoRUN0Yk/p3wN52DOEsL4oBFcLdb76hlpKPtzJIgo67j/XLw=="

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT="$ROOT/ui/vendor"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

fetch() { # name version expected-sha
  curl -fsSL -o "$WORK/$1.tgz" "https://registry.npmjs.org/$1/-/$1-$2.tgz"
  local got="sha512-$(openssl dgst -sha512 -binary "$WORK/$1.tgz" | base64)"
  [ "$got" = "$3" ] || { echo "$1 $2: checksum mismatch ($got)"; exit 1; }
  mkdir -p "$WORK/$1" && tar -xzf "$WORK/$1.tgz" -C "$WORK/$1"
}
fetch pdfjs-dist "$PDFJS" "$PDFJS_SHA"
fetch pdf-lib "$PDFLIB" "$PDFLIB_SHA"

python3 -I - "$WORK/pdfjs-dist/package/build" "$OUT" "$PDFJS" <<'PY'
import re, sys
src, out, version = sys.argv[1], sys.argv[2], sys.argv[3]
head = f"/* pdf.js {version} (Apache-2.0, see pdf.js-LICENSE), turned into a plain script by scripts/vendor-pdf.sh */\n"
def classic(text, exports_to):
    text = text.replace("import.meta.url", '"https://valerie.invalid/pdfjs/"')
    m = re.search(r"export\{([^}]*)\};?\s*$", text)
    if not m:
        sys.exit("no export list at the end")
    body = text[: m.start()]
    if exports_to:
        pairs = []
        for item in m.group(1).split(","):
            parts = item.strip().split(" as ")
            local, name = (parts[0], parts[-1])
            pairs.append(f"{name}:{local}")
        body += f"globalThis.{exports_to}={{{','.join(pairs)}}};"
    return head + "(function(){\"use strict\";\n" + body + "\n})();\n"
main = open(f"{src}/pdf.min.mjs", encoding="utf-8").read()
worker = open(f"{src}/pdf.worker.min.mjs", encoding="utf-8").read()
open(f"{out}/pdf.js", "w", encoding="utf-8").write(classic(main, "pdfjsLib"))
open(f"{out}/pdf.worker.js", "w", encoding="utf-8").write(classic(worker, None))
PY
{ echo "/* pdf-lib $PDFLIB (MIT, see pdf-lib-LICENSE) */"; cat "$WORK/pdf-lib/package/dist/pdf-lib.min.js"; } > "$OUT/pdf-lib.js"
cp "$WORK/pdfjs-dist/package/LICENSE" "$OUT/pdf.js-LICENSE"
cp "$WORK/pdf-lib/package/LICENSE.md" "$OUT/pdf-lib-LICENSE"
echo "pdf.js $PDFJS and pdf-lib $PDFLIB are in ui/vendor"
