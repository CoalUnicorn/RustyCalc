IC_PKG    := ../pkg
ICALC_PKG := ../../../IronCalc/bindings/wasm/pkg
PORT      ?= 8000

.PHONY: sync serve clean

sync:
	@test -d $(IC_PKG)    || (echo "missing $(IC_PKG) — run 'wasm-pack build --target web' in iron-canvas/" && exit 1)
	@test -d $(ICALC_PKG) || (echo "missing $(ICALC_PKG) — run 'make' in IronCalc/bindings/wasm/" && exit 1)
	rm -rf vendor/iron-canvas vendor/ironcalc
	cp -r $(IC_PKG)    vendor/iron-canvas
	cp -r $(ICALC_PKG) vendor/ironcalc

serve: sync
	python3 -m http.server $(PORT)

clean:
	rm -rf vendor/iron-canvas vendor/ironcalc
