mod client;
#[cfg(feature = "demo")]
mod demo;
mod emoji;
mod image;
mod lookalike;
mod model;
mod paths;
mod policy;
mod text;

pub use client::{Client, Events};
#[cfg(feature = "demo")]
pub use demo::Demo;
pub use image::DecodedImage;
pub use lookalike::skeleton;
pub use model::{
    AuthStep, Backend, ChatKind, ChatListId, DeleteOptions, Event, PhotoRef, ReactionId, SafeAccountInfo, SafeChat,
    SafeContent, SafeFile, SafeFolder, SafeMessage, SafeProfile, SafeReaction, SafeReply, SafeTopic, SafeUser, Sender,
    Upload,
};
pub use paths::{log_file, settings_file};
pub use text::{Reason, SafeText, Segment, code_points, is_allowed, sanitize};
