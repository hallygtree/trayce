/// Every failure mode the widget can present to the user.
#[derive(Debug, Clone)]
pub enum WidgetError {
    /// The provider's local logs could not be located, or held no usage
    /// events. Surfaces in the UI as `⚠ logs`.
    LogsNotFound,
    /// Logs exist but their format could not be read (undocumented formats
    /// such as Antigravity's protobuf may change between releases).
    Unreadable(String),
}
