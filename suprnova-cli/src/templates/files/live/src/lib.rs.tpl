pub mod actions;
pub mod bootstrap;
pub mod commands;
pub mod config;
pub mod controllers;
pub mod flash;
pub mod live;
pub mod middleware;
pub mod migrations;
pub mod models;
pub mod routes;
pub mod views;
/// Seed fixtures are linked so bootstrap can register them for `db:seed`.
pub mod seeders;
/// The application scheduler runs retention cleanup alongside user tasks.
pub mod schedule;
