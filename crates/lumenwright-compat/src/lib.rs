//! Holds the `lumenwright` library to the frames a published release draws. The crate has
//! no code; its tests render one request through this tree and through the release as
//! published on crates.io and compare the bytes. It is never published: a published crate
//! cannot depend on one crate version under two names.
