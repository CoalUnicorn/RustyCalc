//! Cursor-context query: is the caret at a position where a cell reference
//! can be inserted?

use ironcalc_base::expressions::{lexer::util::get_tokens, token::TokenType};

/// Returns `true` if `cursor` is at a position in `text` where inserting a
/// cell reference would be syntactically valid.
///
/// Tokenizes the formula up to `cursor` via the ironcalc lexer, skips any
/// trailing [`TokenType::Illegal`] tokens (partial input mid-typing, e.g. the
/// `"B"` in `"=A1+B"`), then checks whether the last meaningful token is an
/// operator or opening delimiter that allows a reference to follow.
pub fn is_in_reference_mode(text: &str, cursor: usize) -> bool {
    if !text.starts_with('=') {
        return false;
    }
    let end = cursor.min(text.len());
    if end <= 1 {
        return true;
    }
    let tokens = get_tokens(&text[..end]);
    // Trailing Illegal tokens represent partial input the user is still typing.
    // Skip them to find the last syntactically complete token before the cursor.
    let last = tokens
        .iter()
        .rev()
        .find(|t| !matches!(t.token, TokenType::Illegal(_) | TokenType::EOF));
    match last {
        None => true,
        Some(t) => matches!(
            t.token,
            TokenType::Addition(_)
                | TokenType::Product(_)
                | TokenType::Compare(_)
                | TokenType::Power
                | TokenType::LeftParenthesis
                | TokenType::Semicolon
                | TokenType::Comma
                | TokenType::And
                | TokenType::Colon
        ),
    }
}
