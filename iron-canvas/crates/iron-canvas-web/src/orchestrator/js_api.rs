use wasm_bindgen::prelude::*;

use super::IronCanvas;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl IronCanvas {
    /// Set the theme from the computed style of a DOM element.
    ///
    /// The method reads the `--palette-*` custom properties.
    #[wasm_bindgen(js_name = "setThemeFromElement")]
    pub fn set_theme_from_element(&mut self, el: &web_sys::Element) {
        self.runtime
            .orchestrator_mut()
            .set_theme(crate::theme_from_element::from_element(el));
        self.restamp_recording_theme();
    }
}

// This API converts Rust query values to the JavaScript wire types.
// The engine enums contain tuple variants.
// Therefore, this API uses the compatible types in `crate::wire`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl IronCanvas {
    /// Find the canvas item at a pixel position in the last painted frame.
    /// `crate::wire::HitTestWire` defines the JavaScript value.
    #[wasm_bindgen(js_name = "hitTest")]
    pub fn hit_test_js(&self, x: f64, y: f64) -> Result<JsValue, JsError> {
        let wire: crate::wire::HitTestWire = self.runtime.orchestrator().hit_test(x, y).into();
        Ok(serde_wasm_bindgen::to_value(&wire)?)
    }

    /// Return the pixel rectangle of a 1-based cell.
    /// Return `null` if the cell is not in the current layout.
    #[wasm_bindgen(js_name = "cellRect")]
    pub fn cell_rect_js(&self, row: i32, column: i32) -> Result<JsValue, JsError> {
        match self.runtime.orchestrator().cell_rect(row, column) {
            Some(rect) => Ok(serde_wasm_bindgen::to_value(&rect)?),
            None => Ok(JsValue::NULL),
        }
    }

    /// Find a resize handle at a pixel position.
    /// `tolerance` is the permitted distance in CSS pixels.
    /// Return `null` if no trailing edge is in this distance.
    #[wasm_bindgen(js_name = "resizeHandleAt")]
    pub fn resize_handle_at_js(&self, x: f64, y: f64, tolerance: f64) -> Result<JsValue, JsError> {
        match self
            .runtime
            .orchestrator()
            .resize_handle_at(x, y, tolerance)
        {
            Some(target) => {
                let wire: crate::wire::ResizeTargetWire = target.into();
                Ok(serde_wasm_bindgen::to_value(&wire)?)
            }
            None => Ok(JsValue::NULL),
        }
    }

    /// Return the pixel position of the autofill handle.
    /// Return `null` if the canvas does not show a selection.
    #[wasm_bindgen(js_name = "autofillHandlePos")]
    pub fn autofill_handle_pos(&self) -> Result<JsValue, JsError> {
        match self.runtime.orchestrator().autofill_handle() {
            Some(p) => Ok(serde_wasm_bindgen::to_value(&p)?),
            None => Ok(JsValue::NULL),
        }
    }

    /// Find the cell at a pixel position without an overlay hit test.
    /// Return `{row, column}` or `null`.
    #[wasm_bindgen(js_name = "pixelToCell")]
    pub fn pixel_to_cell_js(&self, x: f64, y: f64) -> Result<JsValue, JsError> {
        match self.runtime.orchestrator().pixel_to_cell(x, y) {
            Some((row, column)) => {
                let wire = crate::wire::CellCoordWire { row, column };
                Ok(serde_wasm_bindgen::to_value(&wire)?)
            }
            None => Ok(JsValue::NULL),
        }
    }

    /// Return the current drawable size in CSS pixels as `{ w, h }`.
    #[wasm_bindgen(js_name = "canvasSize")]
    pub fn canvas_size_js(&self) -> Result<JsValue, JsError> {
        let wire: crate::wire::CanvasSizeWire = self.runtime.orchestrator().canvas_size().into();
        Ok(serde_wasm_bindgen::to_value(&wire)?)
    }

    /// Measure the required width of a 1-based column in an inclusive row range.
    ///
    /// The host must apply the result to its model and request a repaint.
    /// Return `undefined` if the range has no formatted content.
    /// Also return `undefined` if the model read fails.
    #[wasm_bindgen(js_name = "fitColumnWidth")]
    pub fn fit_column_width_js(&self, column: i32, first_row: i32, last_row: i32) -> Option<f64> {
        self.runtime
            .orchestrator()
            .fit_column_width(column, first_row, last_row)
    }

    // ============================================================
    // Phase 2: overlay setters.
    // ============================================================

    /// Request an overlay repaint without a state change.
    /// Use this method after the host changes model data that the overlay reads.
    #[wasm_bindgen(js_name = "requestOverlayRepaint")]
    pub fn request_overlay_repaint_js(&mut self) {
        self.runtime.orchestrator_mut().request_overlay_repaint();
    }

    /// Report a change to the scroll position, selection, active cell, or sheet.
    /// This method marks the view and overlay in one operation.
    /// The next `renderPending` call selects the applicable paint strategy.
    #[wasm_bindgen(js_name = "viewChanged")]
    pub fn view_changed_js(&mut self) {
        self.runtime.orchestrator_mut().view_changed();
    }

    /// Set the autofill drag target. Pass `null` to clear the target.
    #[wasm_bindgen(js_name = "setExtendTo")]
    pub fn set_extend_to_js(&mut self, target: JsValue) -> Result<(), JsError> {
        let wire: Option<crate::wire::AutofillTargetWire> = serde_wasm_bindgen::from_value(target)?;
        self.runtime
            .orchestrator_mut()
            .set_extend_to(wire.map(Into::into));
        Ok(())
    }

    /// Set the clipboard outline. Pass `null` to clear the outline.
    #[wasm_bindgen(js_name = "setClipboard")]
    pub fn set_clipboard_js(&mut self, area: JsValue) -> Result<(), JsError> {
        let wire: Option<crate::wire::SheetAreaWire> = serde_wasm_bindgen::from_value(area)?;
        self.runtime
            .orchestrator_mut()
            .set_clipboard(wire.map(Into::into));
        Ok(())
    }

    /// Set the formula-entry range highlight. Pass `null` to clear it.
    #[wasm_bindgen(js_name = "setPointRange")]
    pub fn set_point_range_js(&mut self, range: JsValue) -> Result<(), JsError> {
        let wire: Option<crate::wire::RCRangeWire> = serde_wasm_bindgen::from_value(range)?;
        self.runtime
            .orchestrator_mut()
            .set_point_range(wire.map(Into::into));
        Ok(())
    }

    /// Replace the draggable references for the formula.
    #[wasm_bindgen(js_name = "setFormulaRefs")]
    pub fn set_formula_refs_js(&mut self, refs: JsValue) -> Result<(), JsError> {
        let wire: Vec<crate::wire::FormulaRefWire> = serde_wasm_bindgen::from_value(refs)?;
        let refs: Vec<iron_canvas_core::FormulaRef> = wire.into_iter().map(Into::into).collect();
        self.runtime.orchestrator_mut().set_formula_refs(refs);
        Ok(())
    }

    /// Replace all overlay state.
    /// The `Result` permits future validation errors without an API change.
    #[wasm_bindgen(js_name = "setOverlays")]
    pub fn set_overlays_js(&mut self, overlays: JsValue) -> Result<(), JsError> {
        let wire: crate::wire::RenderOverlaysWire = serde_wasm_bindgen::from_value(overlays)?;
        let engine = wire.into_engine().map_err(|msg| JsError::new(&msg))?;
        self.runtime.orchestrator_mut().set_overlays(engine);
        Ok(())
    }

    // ============================================================
    // Phase 3: theme setters.
    // ============================================================

    /// Replace the full theme.
    /// Every palette field is mandatory.
    /// A missing field returns `JsError`.
    /// Use `setThemeVariables` for partial values with a light-theme default.
    /// Use `setThemeFromElement` for a theme from CSS custom properties.
    #[wasm_bindgen(js_name = "setTheme")]
    pub fn set_theme_js(&mut self, theme: JsValue) -> Result<(), JsError> {
        let wire: crate::wire::CanvasThemeWire = serde_wasm_bindgen::from_value(theme)?;
        self.runtime.orchestrator_mut().set_theme(wire.into());
        self.restamp_recording_theme();
        Ok(())
    }

    /// Set optional theme values.
    /// A missing value uses the applicable value from `CanvasTheme::light()`.
    #[wasm_bindgen(js_name = "setThemeVariables")]
    pub fn set_theme_variables_js(&mut self, vars: JsValue) -> Result<(), JsError> {
        let wire: crate::wire::ThemeVariablesWire = serde_wasm_bindgen::from_value(vars)?;
        self.runtime
            .orchestrator_mut()
            .set_theme_variables(wire.into());
        self.restamp_recording_theme();
        Ok(())
    }
}
