IC_CRATE  := iron-canvas-web
IC_DIR    := ../crates/$(IC_CRATE)
IC_PKG    := $(IC_DIR)/pkg
ICALC_PKG := ../../../IronCalc/bindings/wasm/pkg
PORT      ?= 8000
# dev | release | profiling
PROFILE   ?= release

PROFILE_FLAG := $(if $(filter dev,$(PROFILE)),--dev,$(if $(filter profiling,$(PROFILE)),--profiling,--release))

.PHONY: build sync serve clean

build:
	cd $(IC_DIR) && wasm-pack build --target web $(PROFILE_FLAG)

sync:
	@test -d $(IC_PKG)    || (echo "missing $(IC_PKG) — run 'make build' first" && exit 1)
	@test -d $(ICALC_PKG) || (echo "missing $(ICALC_PKG) — run 'make' in IronCalc/bindings/wasm/" && exit 1)
	rm -rf vendor/iron-canvas vendor/ironcalc
	cp -r $(IC_PKG)    vendor/iron-canvas
	cp -r $(ICALC_PKG) vendor/ironcalc

serve: sync
	python3 -m http.server $(PORT)

clean:
	rm -rf vendor/iron-canvas vendor/ironcalc
