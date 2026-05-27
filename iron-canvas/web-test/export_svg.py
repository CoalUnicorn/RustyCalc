#!/usr/bin/env python3
"""export_svg.py - Render an .xlsx through iron-canvas and export as SVG.

Uses Playwright to drive the full RustyCalc app, upload the workbook,
wait for canvas render, and capture the SVG export.

Usage:
    python3 export_svg.py input.xlsx output.svg [--width 1200] [--height 800]
"""

import argparse
import base64
import os
import subprocess
import sys
import time
from pathlib import Path


def serve_dist(port):
    """Start the RustyCalc dist server with path rewrite."""
    import http.server
    import threading

    dist = Path("/home/mmm/01_Dev/RustyCalc/dist")

    class Handler(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, directory=str(dist), **kwargs)

        def do_GET(self):
            if self.path.startswith("/RustyCalc/"):
                self.path = self.path[len("/RustyCalc"):]
            elif self.path == "/RustyCalc":
                self.path = "/"
            elif self.path == "/":
                self.send_response(301)
                self.send_header("Location", "/RustyCalc/")
                self.end_headers()
                return
            super().do_GET()

    server = http.server.HTTPServer(("", port), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server


def main():
    parser = argparse.ArgumentParser(description="Export .xlsx to SVG via iron-canvas")
    parser.add_argument("input", type=Path, help="Input .xlsx file")
    parser.add_argument("output", type=Path, help="Output .svg file")
    parser.add_argument("--width", type=int, default=1200)
    parser.add_argument("--height", type=int, default=800)
    parser.add_argument("--sheet", type=int, default=None)
    parser.add_argument("--app-url", default=None)
    args = parser.parse_args()

    if not args.input.exists():
        print(f"Error: {args.input} not found", file=sys.stderr)
        sys.exit(1)

    xlsx_data = args.input.read_bytes()
    xlsx_b64 = base64.b64encode(xlsx_data).decode()
    output_path = str(args.output.resolve())

    # Start server if needed
    server = None
    if args.app_url:
        app_url = args.app_url
    else:
        port = 8090
        server = serve_dist(port)
        app_url = f"http://localhost:{port}/RustyCalc/"
        time.sleep(0.5)

    # Sheet selection JS
    sheet_js = ""
    if args.sheet is not None:
        sheet_js = (
            f"const tabs = await page.$$('[data-sheet-tab]');\n"
            f"if (tabs[{args.sheet}]) await tabs[{args.sheet}].click();\n"
            f"await page.waitForTimeout(1000);\n"
        )

    # Build Playwright script
    pw_script = """const { chromium } = require('playwright');
const fs = require('fs');

(async () => {
    const browser = await chromium.launch({ headless: true });
    const page = await browser.newPage();
    
    await page.goto('""" + app_url + """');
    await page.waitForSelector('canvas', { timeout: 30000 });
    await page.waitForTimeout(2000);
    
    // Upload the xlsx file
    const xlsxBuffer = Buffer.from('""" + xlsx_b64 + """', 'base64');
    const fileInput = await page.$('input[accept=".xlsx"]');
    await fileInput.setInputFiles({
        name: '""" + args.input.name + """',
        mimeType: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
        buffer: xlsxBuffer
    });
    
    await page.waitForTimeout(3000);
""" + sheet_js + """
    // Click SVG export and capture download
    const downloadPromise = page.waitForEvent('download', { timeout: 10000 });
    await page.locator('button:has-text("⇩ SVG")').click();
    const download = await downloadPromise;
    
    // Read the download stream
    const stream = await download.createReadStream();
    const chunks = [];
    for await (const chunk of stream) {
        chunks.push(chunk);
    }
    const svg = Buffer.concat(chunks).toString('utf8');
    fs.writeFileSync('""" + output_path + """', svg);
    console.log('SVG exported: ' + svg.length + ' bytes');
    
    await browser.close();
})();
"""

    pw_path = Path("/tmp/export_svg_pw.js")
    pw_path.write_text(pw_script)

    node_path = "/home/mmm/.nvm/versions/node/v24.9.0/lib/node_modules"
    result = subprocess.run(
        ["node", str(pw_path)],
        capture_output=True, text=True,
        timeout=60,
        env={**os.environ, "NODE_PATH": node_path},
    )

    if server:
        server.shutdown()

    if result.returncode != 0:
        print(f"Error: {result.stderr}", file=sys.stderr)
        sys.exit(1)

    print(result.stdout.strip())
    if args.output.exists():
        print(f"Saved: {args.output} ({args.output.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
