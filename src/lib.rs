pub mod reminder;
pub mod task;

pub const DATETIME_FMT: &'static str = "%d.%m.%Y %H:%M";

pub type LocalDT = chrono::DateTime<chrono::Local>;

pub use reminder::Reminder;
pub use task::Task;

pub fn import_datetime(x: i64) -> LocalDT {
    chrono::DateTime::from_timestamp(x, 0)
        .unwrap()
        .with_timezone(&chrono::Local)
}
