IC_CRATE  := iron-canvas-web
IC_DIR    := ../crates/$(IC_CRATE)
IC_PKG    := $(IC_DIR)/pkg
ICALC_PKG := ../../IronCalc/bindings/wasm/pkg
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

.PHONY: build sync serve clean datagrid datagrid-serve

build:
	cd $(IC_DIR) && wasm-pack build --target web $(PROFILE_FLAG) --no-opt $(FEATURES_FLAG)

sync:
	@test -d $(IC_PKG)    || (echo "missing $(IC_PKG) — run 'make build' first" && exit 1)
	@test -d $(ICALC_PKG) || (echo "missing $(ICALC_PKG) — run 'make' in IronCalc/bindings/wasm/" && exit 1)
	rm -rf vendor/iron-canvas vendor/ironcalc
	cp -r $(IC_PKG)    vendor/iron-canvas
	cp -r $(ICALC_PKG) vendor/ironcalc

serve: sync
	python3 -m http.server $(PORT)

clean:
	rm -rf vendor/iron-canvas vendor/ironcalc vendor/iron-canvas-datagrid

datagrid:
	cd $(DG_DIR) && wasm-pack build --target web $(PROFILE_FLAG) --no-opt
	rm -rf vendor/iron-canvas-datagrid
	cp -r $(DG_PKG) vendor/iron-canvas-datagrid
	# No ironcalc vendor needed: the datagrid bundle is engine-agnostic.

datagrid-serve: datagrid
	python3 -m http.server $(PORT)
