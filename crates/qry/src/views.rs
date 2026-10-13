//! Drawing code that reads state and a [`Theme`](crate::theme::Theme) and
//! owns nothing. T101 moves each component's `draw` here as a free
//! function; for now it holds only the theme swatch, a test-only view.

#[cfg(test)]
mod swatch;
