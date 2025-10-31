use chrono::{Local, NaiveDate, NaiveTime, TimeDelta};
use clap::{Parser, Subcommand};
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::Null;

use rem::{import_datetime, LocalDT, Reminder, Task, DATETIME_FMT};

const DATABASE_FILE: &'static str = "db.sqlite";
const HOME_DIR: &'static str = "rem";
const DATABASE_NAME: &'static str = "main";

#[derive(Clone, PartialEq, Eq, Debug, Subcommand)]
enum Action {
    #[command(about = "Display tasks")]
    Tasks {
        #[arg(short, long, help = "show all tasks, including completed ones")]
        all: bool,

        #[arg(short, long, help = "show all information on the tasks")]
        verbose: bool,
    },
    #[command(about = "Record a bit of work for a task")]
    Record {
        #[arg(help = "task id to record a work bit for")]
        task_id: u64,
        #[arg(help = "optional description of the work bit")]
        description: Option<String>,
    },
    #[command(about = "Create a task")]
    Task {
        #[arg(help = "task title")]
        title: String,
        #[arg(help = "optional detailed task description")]
        description: Option<String>,
        #[arg(short, long, help = "optional due date/time as DD.MM.YYYY [HH:MM]")]
        due: Option<String>,
        #[arg(short, long, help = "optional scheduled start as DD.MM.YYYY [HH:MM]")]
        start: Option<String>,
    },
    #[command(about = "Delete a task")]
    DeleteTask {
        #[arg(help = "id of the task to delete")]
        id: u64,
    },
    #[command(about = "Mark a task as completed")]
    Complete {
        #[arg(help = "id of the task to mark completed")]
        id: u64,
    },
    #[command(about = "Add a generator for recurring events")]
    Reminder {
        #[arg(help = "title")]
        title: String,
        #[arg(help = "first due date")]
        first_due: String,
        #[arg(help = "recurrence period")]
        period: String,
        #[arg(long, short, help = "optional description")]
        description: Option<String>,
        #[arg(long, short, help = "last occurrence is before this datetime")]
        until: Option<String>,
    },
    #[command(about = "Display reminders")]
    Reminders {
        #[arg(short, long, help = "show all reminders, including inactive ones")]
        all: bool,

        #[arg(short, long, help = "show all information on the reminders")]
        verbose: bool,
    },
    #[command(about = "Stop a reminder from generating new tasks")]
    Stop { id: u64 },
}

#[derive(Parser, Debug)]
struct Args {
    #[command(subcommand)]
    action: Action,
}

struct App {
    conn: rusqlite::Connection,
    now: LocalDT,
}

impl App {
    fn try_init(conn: rusqlite::Connection) -> Result<Self, String> {
        let now = chrono::Local::now();

        if !conn.table_exists(Some(DATABASE_NAME), "reminders").unwrap() {
            let _ = conn
                .execute(
                    "CREATE TABLE IF NOT EXISTS reminders (
                      id INTEGER PRIMARY KEY,
                      title TEXT NOT NULL,
                      description TEXT,
                      created INTEGER NOT NULL,
                      first_due INTEGER NOT NULL,
                      period INTEGER NOT NULL,
                      until INTEGER
                    );",
                    [],
                )
                .map_err(|err| format!("could not create reminders table: {err}"))?;
        }

        if !conn.table_exists(Some(DATABASE_NAME), "tasks").unwrap() {
            let _ = conn
                .execute(
                    "CREATE TABLE IF NOT EXISTS tasks (
                      id INTEGER PRIMARY KEY,
                      title TEXT NOT NULL,
                      description TEXT,
                      created INTEGER NOT NULL,
                      start INTEGER,
                      due INTEGER,
                      generated_by INTEGER,
                      FOREIGN KEY(generated_by) REFERENCES reminders(id),
                      completed INTEGER
                    );",
                    [],
                )
                .map_err(|err| format!("could not create tasks table: {err}"))?;
        }

        if !conn.table_exists(Some(DATABASE_NAME), "work_bits").unwrap() {
            let _ = conn
                .execute(
                    "CREATE TABLE IF NOT EXISTS work_bits (
                      id INTEGER PRIMARY KEY,
                      task_id INTEGER NOT NULL,
                      FOREIGN KEY(task_id) REFERENCES tasks(id),
                      datetime INTEGER NOT NULL,
                      description TEXT
                    );",
                    [],
                )
                .map_err(|err| format!("could not create work_bits table: {err}"))?;
        }

        Ok(Self { conn, now })
    }

    fn add_task(
        &mut self,
        title: String,
        description: Option<String>,
        start: Option<LocalDT>,
        due: Option<LocalDT>,
        generated_by: Option<u64>,
    ) -> Result<(), String> {
        let _ = self.conn.execute(
            "INSERT INTO tasks (title, description, created, start, due, completed, generated_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                title.clone(),
                description.to_owned(),
                start.map(|t| t.timestamp()),
                self.now.timestamp(),
                due.map(|t| t.timestamp()),
                Null,
                generated_by,
            ),
        ).map_err(|err| { format!("could not insert task: {err}") })?;
        Ok(())
    }

    fn reminders_to_tasks(&mut self) -> Result<(), String> {
        let reminders = {
            let mut res = self
            .conn
            .prepare(
                "SELECT id, title, description, created, first_due, period, until FROM reminders where until is NULL or until > ?1;",
            )
            .map_err(|err| format!("could not query tasks: {err}"))?;

            let s = res
                .query([self.now.timestamp()])
                .map_err(|err| format!("Could not query database: {err}"))?
                .map(|row| Reminder::from_db_row(row))
                .collect::<Vec<_>>()
                .map_err(|err| format!("Could not acquire reminders from database: {err}"))?;
            drop(res);
            s
        };

        for reminder in reminders.iter() {
            let generated_tasks = {
                let mut r = self
                    .conn
                    .prepare("SELECT * FROM tasks where generated_by == ?1;")
                    .map_err(|err| format!("could not query tasks: {err}"))?;

                let mut generated_tasks = r
                    .query([reminder.id])
                    .map_err(|err| format!("Could not query database: {err}"))?
                    .map(|row| Task::from_db_row(row, None))
                    .collect::<Vec<Task>>()
                    .map_err(|err| {
                        format!("Could not find tasks corresponding to reminder: {err}")
                    })?;

                generated_tasks.sort_by_cached_key(|x| {
                    x.due
                        .expect("elements of recurring sequence need to have due date")
                });

                generated_tasks
            };

            let mut next_due = reminder.first_due;
            while next_due < self.now + reminder.period {
                // insert a new task if the instance at next_due is missing from
                // the list of tasks associated with this list of generated tasks
                if generated_tasks
                    .iter()
                    .find(|task| {
                        let due = task.due.expect("Recurring tasks need to have a due date");
                        due == next_due
                    })
                    .is_none()
                {
                    self.add_task(
                        reminder.title.clone(),
                        reminder.description.to_owned(),
                        Some(next_due - reminder.period),
                        Some(next_due),
                        Some(reminder.id),
                    )?;
                }

                next_due += reminder.period;
            }
        }

        Ok(())
    }

    fn add_reminder(
        &mut self,
        title: String,
        description: Option<String>,
        first_due: LocalDT,
        period: TimeDelta,
        until: Option<LocalDT>,
    ) -> Result<(), String> {
        let until = until.map(|x| x.timestamp());
        self.conn.execute(
            "INSERT INTO reminders (title, description, first_due, period, until, created) values (?1, ?2, ?3, ?4, ?5, ?6);",
            (title, description, first_due.timestamp(), period.num_seconds(), until, self.now.timestamp())
        ).map_err(|err| format!("Could not add reminder: {err}"))?;

        Ok(())
    }

    fn show_reminders(&self, all: bool, verbose: bool) -> Result<(), String> {
        let mut res = self
            .conn
            .prepare(
                "SELECT id, title, description, created, first_due, period, until FROM reminders;",
            )
            .map_err(|err| format!("could not query tasks: {err}"))?;

        let rows = res
            .query([])
            .map_err(|err| format!("Could not query database: {err}"))?
            .map(|row| Reminder::from_db_row(row))
            .iterator();

        for row in rows {
            let r = match row {
                Ok(row) => row,
                Err(err) => return Err(format!("Error querying database: {err}")),
            };
            print!("{}", r.display(all, verbose, self.now));
        }

        Ok(())
    }

    fn stop_reminder(&mut self, id: u64) -> Result<(), String> {
        let until = self.now;
        self.conn
            .execute(
                "UPDATE reminders SET until = ?1 WHERE id = ?2",
                (until.timestamp(), id),
            )
            .map_err(|err| format!("Could stop reminder: {err}"))?;

        Ok(())
    }

    fn show_tasks(&self, all: bool, verbose: bool) -> Result<(), String> {
        let mut res = self
            .conn
            .prepare("SELECT * FROM tasks;")
            .map_err(|err| format!("Could not query tasks: {err}"))?;

        let rows = res
            .query([])
            .map_err(|err| format!("Could not query database: {err}"))?
            .map(|row| Task::from_db_row(row, Some(&self.conn)))
            .iterator();

        for row in rows {
            let t = match row {
                Ok(row) => row,
                Err(err) => return Err(format!("Error querying database: {err}")),
            };
            print!("{}", t.display(all, verbose, self.now));
        }
        Ok(())
    }

    fn delete_task(&mut self, id: u64) -> Result<(), String> {
        let res = self
            .conn
            .execute("DELETE FROM tasks where ID = ?1", [id])
            .map_err(|err| format!("could not query tasks: {err}"))?;

        if res == 0 {
            Err(format!("Could not delete Task. ID not found."))
        } else {
            Ok(())
        }
    }

    fn complete_task(&self, id: u64) -> Result<(), String> {
        let completed: Option<i64> = self
            .conn
            .query_one("select completed from tasks where id = ?1", (id,), |row| {
                row.get(0)
            })
            .map_err(|err| format!("Could not get task completion status: {err}"))?;
        let completed = completed.map(import_datetime);

        if let Some(completed) = completed {
            return Err(format!(
                "Could not mark task {id} as completed. Already completed at {completed}",
                completed = completed.format(DATETIME_FMT)
            ));
        }

        let res = self
            .conn
            .execute(
                "UPDATE tasks SET completed = ?1 where id = ?2;",
                (self.now.timestamp(), id),
            )
            .map_err(|err| format!("Could not mark task {id} as completed: {err}"))?;

        assert_eq!(res, 1);

        Ok(())
    }

    fn add_work_bit(&self, task_id: u64, description: Option<String>) -> Result<(), String> {
        if let Some(description) = description {
            let res = self
                .conn
                .execute(
                    "INSERT INTO work_bits (task_id, datetime, description) values (?1, ?2, ?3);",
                    (task_id, self.now.timestamp(), description),
                )
                .map_err(|err| err.to_string())?;
            assert_eq!(res, 1);
        } else {
            let res = self
                .conn
                .execute(
                    "INSERT INTO work_bits (task_id, datetime) values (?1, ?2);",
                    (task_id, self.now.timestamp()),
                )
                .map_err(|err| err.to_string())?;
            assert_eq!(res, 1);
        }

        Ok(())
    }
}

fn get_database_connection() -> Result<rusqlite::Connection, String> {
    let mut path = match std::env::var("XDG_DATA_HOME") {
        Ok(v) => std::path::PathBuf::from(v),
        Err(v) => match v {
            std::env::VarError::NotPresent => std::env::home_dir()
                .map(|mut x| {
                    x.push(".local");
                    x.push("share");
                    x
                })
                .ok_or(format!("Could not determine home directory"))?,
            std::env::VarError::NotUnicode(_) => {
                return Err(format!(
                    "Could not get config home directory. Returned string was not unicode."
                ));
            }
        },
    };
    path.push(HOME_DIR);

    if !path.exists() {
        std::fs::create_dir_all(&path)
            .map_err(|err| format!("Could not create data directory: {err}"))?;
    } else {
        if path.is_file() {
            return Err(format!("Could not get data directory. Is a file."));
        }
    };
    path.push(DATABASE_FILE);

    // TODO: handle the error properly
    Ok(rusqlite::Connection::open(path)
        .map_err(|err| format!("Could not open database connection: {err}"))?)
}

/// Parse a duration expression with weeks and days
///
/// parsing examples:
/// '1w 2d' => TimeDelta()
///
/// * `repr`: timedelta to parse
fn parse_timedelta(repr: impl AsRef<str>) -> Result<TimeDelta, String> {
    let mut weeks = None;
    let mut days = None;
    for part in repr.as_ref().trim().split(' ') {
        let bytes = part.as_bytes();
        let idx = bytes.iter().take_while(|x| x.is_ascii_digit()).count();
        let (num, desc) = bytes.split_at(idx);
        if desc.len() > 1 {
            return Err(format!(
                "invalid duration specifier '{desc}'. Expected 'w' or 'd'.",
                desc = std::str::from_utf8(desc).expect("rest of input is utf8")
            ));
        }
        let desc = desc[0];

        let num = std::str::from_utf8(num).expect("used is_ascii_digit to find end of num");
        let num = num
            .parse::<i64>()
            .map_err(|err| format!("Could not parse number from '{num}': {err}"))?;

        match desc as char {
            'w' => {
                if let Some(weeks) = weeks {
                    return Err(format!("Cannot specify weeks twice. Already got {weeks}."));
                } else {
                    weeks = Some(num);
                }
            }
            'd' => {
                if let Some(days) = days {
                    return Err(format!("Cannot specify days twice. Already got {days}."));
                } else {
                    days = Some(num);
                }
            }
            _ => {
                return Err(format!(
                    "Invalid duration specifier '{desc}.' Expected 'w' or 'd'."
                ))
            }
        }
    }

    if weeks.is_none() && days.is_none() {
        return Err(format!(
            "Need to specify either number of days or number of weeks."
        ));
    }

    let days = days.map(TimeDelta::days).unwrap_or(TimeDelta::days(0));
    let weeks = weeks.map(TimeDelta::days).unwrap_or(TimeDelta::days(0)) * 7;

    Ok(days + weeks)
}

fn parse_date_time(repr: impl AsRef<str>) -> Result<LocalDT, String> {
    if let Some((date, time)) = repr.as_ref().split_once(" ") {
        let date = NaiveDate::parse_from_str(date, "%d.%m.%Y")
            .map_err(|err| format!("Could not parse date: {err}"))?;
        let time = NaiveTime::parse_from_str(time, "%H:%M")
            .map_err(|err| format!("Could not parse time: {err}"))?;
        let dt = date.and_time(time).and_local_timezone(Local).unwrap();
        Ok(dt)
    } else {
        let date = NaiveDate::parse_from_str(repr.as_ref(), "%d.%m.%Y")
            .map_err(|err| format!("Could not parse date: {err}"))?;
        let dt = date
            .and_hms_opt(8, 0, 0)
            .expect("valid time")
            .and_local_timezone(Local)
            .unwrap();
        Ok(dt)
    }
}

fn main() {
    let conn = get_database_connection().unwrap_or_else(|err| {
        eprintln!("Could not get database connection: {err}");
        std::process::exit(1);
    });

    let mut app = App::try_init(conn).unwrap_or_else(|err| {
        eprintln!("ERROR: could not initialize application: {err}");
        std::process::exit(1);
    });

    app.reminders_to_tasks()
        .unwrap_or_else(|err| eprintln!("ERROR: Could not convert tasks to reminders: {err}"));

    match Args::parse().action {
        Action::Tasks { all, verbose } => {
            app.show_tasks(all, verbose).unwrap_or_else(|err| {
                eprintln!("Could not show tasks: {err}");
                std::process::exit(1);
            });
        }
        Action::Task {
            title,
            description,
            due,
            start,
        } => {
            let due = due.map(parse_date_time).map(|x| {
                x.unwrap_or_else(|err| {
                    eprintln!("Could not parse due datetime: {}", err);
                    std::process::exit(1);
                })
            });

            let start = start.map(parse_date_time).map(|x| {
                x.unwrap_or_else(|err| {
                    eprintln!("Could not parse start datetime: {}", err);
                    std::process::exit(1);
                })
            });

            app.add_task(title, description, start, due, None)
                .unwrap_or_else(|err| {
                    eprintln!("ERROR: could not add task: {err}");
                    std::process::exit(1);
                });
        }
        Action::DeleteTask { id } => {
            app.delete_task(id).unwrap_or_else(|err| {
                eprintln!("ERROR: could not delete task: {err}");
                std::process::exit(1);
            });
        }
        Action::Complete { id } => {
            app.complete_task(id).unwrap_or_else(|err| {
                eprintln!("ERROR: could not delete task: {err}");
                std::process::exit(1);
            });
        }
        Action::Reminder {
            title,
            description,
            first_due,
            period,
            until,
        } => {
            let first_due = parse_date_time(first_due).unwrap_or_else(|err| {
                eprintln!("Could not parse first due date: {}", err);
                std::process::exit(1);
            });
            let until = until.map(|x| {
                parse_date_time(x).unwrap_or_else(|err| {
                    eprintln!("Could not parse until time: {}", err);
                    std::process::exit(1);
                })
            });

            let period = parse_timedelta(period).unwrap_or_else(|err| {
                eprintln!("Could not parse period: {err}");
                std::process::exit(1);
            });

            app.add_reminder(title, description, first_due, period, until)
                .unwrap_or_else(|err| {
                    eprintln!("Could not add reminder: {err}");
                    std::process::exit(1);
                });
        }
        Action::Reminders { all, verbose } => {
            app.show_reminders(all, verbose).unwrap_or_else(|err| {
                eprintln!("Could not show reminders: {err}");
                std::process::exit(1)
            });
        }
        Action::Stop { id } => {
            app.stop_reminder(id).unwrap_or_else(|err| {
                eprintln!("Could not stop reminder: {err}");
                std::process::exit(1)
            });
        }
        Action::Record {
            task_id,
            description,
        } => app
            .add_work_bit(task_id, description)
            .unwrap_or_else(|err| {
                eprintln!("Could not record work: {err}");
                std::process::exit(1);
            }),
    }
}

#[cfg(test)]
mod test {
    use rusqlite::Connection;

    use super::*;
    #[test]
    fn db() {
        let conn = Connection::open_in_memory().unwrap();

        conn.execute(
            "CREATE TABLE IF NOT EXISTS tasks (
                id INTEGER PRIMARY KEY,
                title TEXT,
                description TEXT,
                created INTEGER,
                due INTEGER
        );",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO tasks (title, created) values (?1, ?2)",
            ["Foo".to_string(), 0.to_string()],
        )
        .unwrap();

        let mut stmt = conn.prepare("select * from tasks").unwrap();
        let _ = stmt
            .query_map([], |row| {
                let _: Option<i64> = row.get_unwrap("due");
                Ok(())
            })
            .unwrap()
            .count();
    }

    #[test]
    fn test_show_tasks() {
        let conn = Connection::open_in_memory().unwrap();
        let mut app = App::try_init(conn).unwrap();

        app.add_task("Test".to_string(), None, None, None, None)
            .expect("adding task");

        app.show_tasks(false, true).unwrap();
    }

    #[test]
    fn parse_timedelta_week() {
        assert_eq!(parse_timedelta("1w"), Ok(TimeDelta::days(7)));
    }

    #[test]
    fn parse_timedelta_day() {
        assert_eq!(parse_timedelta("1d"), Ok(TimeDelta::days(1)));
    }

    #[test]
    fn parse_timedelta_fail() {
        assert!(parse_timedelta("1wf 2d").is_err());
        assert!(parse_timedelta("1w 1w").is_err());
        assert!(parse_timedelta("1d 1d").is_err());
    }

    #[test]
    fn parse_timedelta_mixed() {
        assert_eq!(parse_timedelta("1w 2d"), Ok(TimeDelta::days(9)));
        assert_eq!(parse_timedelta("2w 1d"), Ok(TimeDelta::days(15)));
    }
}
