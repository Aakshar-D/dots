//! Dots that make dots (dots-make-dots design): what a maker may give a dot it makes (`reach`),
//! what a maker's run does (`service`), how it lists, changes and deletes the dots it made
//! (`manage`), what waits for the person's answer (`requests`), the log of it (`log`), how a
//! maker is woken (`wake`), and the jobs it gives to the dots it made (`jobs`).

pub mod jobs;
pub mod log;
pub mod manage;
pub mod reach;
pub mod requests;
pub mod service;
pub mod wake;
