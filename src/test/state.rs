use crate::Owner;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn status_initializes_to_none() {
    let owner = Owner::new();
    owner.with(|| {
        let state = crate::state::WorkbookState::new(crate::events::EventBus::new());
        assert_eq!(state.status.get_untracked(), None);
    });
}
