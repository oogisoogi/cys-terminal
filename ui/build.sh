#!/bin/sh
# Bundle the aiterm UI into ui/dist (plain static files; no dev server).
set -e
cd "$(dirname "$0")"
bun install --silent
mkdir -p dist
bun build src/main.ts --outdir dist --target browser --minify
cp index.html dist/index.html
cp src/style.css dist/style.css
cp node_modules/@xterm/xterm/css/xterm.css dist/xterm.css
echo "ui/dist ready"
