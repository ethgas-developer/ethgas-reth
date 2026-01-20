//! Traits describing node builder extensions.

use std::fmt::Debug;

use crate::builder::EthgasBuilder;


/// Customizes the node builder before launch.
///
/// Register extensions via [`EthgasNodeRunner::install_ext`].
pub trait EthgasNodeExtension: Send + Sync + Debug {
    /// Applies the extension to the supplied builder.
    fn apply(self: Box<Self>, builder: EthgasBuilder) -> EthgasBuilder;
}

/// An extension that can be built from a config.
pub trait FromExtensionConfig: EthgasNodeExtension + Sized {
    /// Configuration type used to construct this extension.
    type Config;

    /// Creates a new extension from the provided configuration.
    fn from_config(config: Self::Config) -> Self;
}
