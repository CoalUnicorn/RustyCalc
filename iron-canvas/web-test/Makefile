IC_CRATE  := iron-canvas-web
IC_DIR    := ../crates/$(IC_CRATE)
IC_PKG    := $(IC_DIR)/pkg
ICALC_PKG := ../../IronCalc/bindings/wasm/pkg
ICALC_XLSX_MANIFEST := ../../IronCalc/xlsx/Cargo.toml
DEMO_XLSX := $(wildcard demo/*.xlsx)
DEMO_ICALC := $(DEMO_XLSX:.xlsx=.ic)
PORT      ?= 8000
# dev | release | profiling
PROFILE   ?= release
# Space-separated cargo features; `dev-tools` enables the .icr recorder
# (startRecording / stopRecording on IronCanvas) plus the replay exports
# (icrReplayGridOps / icrReplayOverlayOps) the viewer page consumes.
FEATURES  ?=

PROFILE_FLAG  := $(if $(filter dev,$(PROFILE)),--dev,$(if $(filter profiling,$(PROFILE)),--profiling,--release))
FEATURES_FLAG := $(if $(strip $(FEATURES)),--features "$(FEATURES)",)

DG_CRATE := iron-canvas-datagrid-web
DG_DIR   := ../crates/$(DG_CRATE)
DG_PKG   := $(DG_DIR)/pkg

.PHONY: build demos sync serve check browser-test browser-test-all clean datagrid datagrid-serve

build:
	cd $(IC_DIR) && wasm-pack build --target web $(PROFILE_FLAG) --no-opt $(FEATURES_FLAG)

demos: $(DEMO_ICALC)

$(ICALC_PKG)/wasm_bg.wasm:
	@echo "missing $@ — run 'make' in IronCalc/bindings/wasm/"
	@exit 1

demo/%.ic: demo/%.xlsx $(ICALC_PKG)/wasm_bg.wasm
	rm -f "$@"
	cargo run --quiet --manifest-path $(ICALC_XLSX_MANIFEST) --bin xlsx_2_icalc -- "$<" "$@"

sync: demos
	@test -d $(IC_PKG)    || (echo "missing $(IC_PKG) — run 'make build' first" && exit 1)
	@test -d $(ICALC_PKG) || (echo "missing $(ICALC_PKG) — run 'make' in IronCalc/bindings/wasm/" && exit 1)
	mkdir -p vendor
	rm -rf vendor/iron-canvas vendor/ironcalc
	cp -r $(IC_PKG)    vendor/iron-canvas
	cp -r $(ICALC_PKG) vendor/ironcalc

serve: sync
	python3 -m http.server $(PORT)

check:
	node --check harness.js
	node --check harness-core.js
	node --check browser-smoke.mjs
	node --test harness-core.test.mjs

browser-test: sync check
	node browser-smoke.mjs

browser-test-all: sync check
	WORKBOOK=dynamic_arrays node browser-smoke.mjs
	WORKBOOK=forensics node browser-smoke.mjs
	WORKBOOK=sales_dashboard node browser-smoke.mjs

clean:
	rm -rf vendor/iron-canvas vendor/ironcalc vendor/iron-canvas-datagrid
	rm -f $(DEMO_ICALC)

datagrid:
	cd $(DG_DIR) && wasm-pack build --target web $(PROFILE_FLAG) --no-opt
	mkdir -p vendor
	rm -rf vendor/iron-canvas-datagrid
	cp -r $(DG_PKG) vendor/iron-canvas-datagrid
	# No ironcalc vendor needed: the datagrid bundle is engine-agnostic.

datagrid-serve: datagrid
	python3 -m http.server $(PORT)
