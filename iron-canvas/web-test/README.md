# web-test

Manual smoke harness for the `JsBackedModel` JS bridge. Paints a three-sheet
workbook through the real iron-canvas pipeline and console-logs a snapshot
of every bridge round-trip on first paint.

One HTML file, vanilla JS, no framework, no bundler.

## Build prerequisites

Change `Makefile` `ICALC_PKG` location to your IronCalc directory.
Or override `ICALC_PKG` with your IronCalc path.
```sh
make serve ICALC_PKG=/path/to/IronCalc/bindings/wasm/pkg
```

```sh
# Build wasm, create vendor directory with bindings 
make sync

# python3 http server
make serve

# Delete vendor
make clean
```


Open: <http://localhost:8000/>
