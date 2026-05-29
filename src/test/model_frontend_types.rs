    use crate::model::frontend_types::*;

    #[test]
    fn css_color_new_non_empty() {
        let c = CssColor::new("#FF0000");
        assert_eq!(c.as_str(), "#ff0000");
    }

    #[test]
    fn css_color_new_empty_substitutes_black() {
        let c = CssColor::new("");
        assert_eq!(c.as_str(), "#000000");
    }

    #[test]
    fn safe_font_family_known_names() {
        assert_eq!(SafeFontFamily::from(Some("Arial")), SafeFontFamily::Arial);
        assert_eq!(
            SafeFontFamily::from(Some("Calibri")),
            SafeFontFamily::CalibriLike
        );
        assert_eq!(
            SafeFontFamily::from(Some("Courier New")),
            SafeFontFamily::CourierNew
        );
        assert_eq!(
            SafeFontFamily::from(Some("Times New Roman")),
            SafeFontFamily::TimesNewRoman
        );
    }

    #[test]
    fn safe_font_family_unknown_falls_back() {
        assert_eq!(
            SafeFontFamily::from(Some("Wingdings")),
            SafeFontFamily::SystemUi
        );
        assert_eq!(SafeFontFamily::from(None), SafeFontFamily::SystemUi);
    }

    #[test]
    fn safe_font_family_css_names() {
        assert_eq!(SafeFontFamily::Arial.css_name(), "Arial");
        assert_eq!(SafeFontFamily::CourierNew.css_name(), "Courier New");
        assert_eq!(SafeFontFamily::SystemUi.css_name(), "system-ui");
    }
