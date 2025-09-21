use chrono::{Local, NaiveDate, NaiveTime};
use clap::{Parser, Subcommand};
use colored::Colorize;
use rusqlite::types::Null;

const DATABASE_FILE: &'static str = "db.sqlite";
const HOME_DIR: &'static str = "rem";
const DATABASE_NAME: &'static str = "main";
const DATETIME_FMT: &'static str = "%d.%m.%Y %H:%M";

#[derive(Clone, PartialEq, Eq, Debug, Subcommand)]
enum Action {
    Show {
        #[arg(short, long)]
        all: bool,

        #[arg(short, long)]
        verbose: bool,
    },
    Task {
        #[arg(help = "task title")]
        title: String,
        #[arg(help = "optional detailed task description")]
        description: Option<String>,
        #[arg(short, long, help = "optional due date/time as DD.MM.YYYY [HH:MM]")]
        due: Option<String>,
    },
    DeleteTask {
        id: u64,
    },
    Complete {
        id: u64,
    },
}

#[derive(Parser, Debug)]
struct Args {
    #[command(subcommand)]
    action: Action,
}

struct App {
    conn: rusqlite::Connection,
}

impl App {
    fn try_init(conn: rusqlite::Connection) -> Result<Self, String> {
        if !conn.table_exists(Some(DATABASE_NAME), "tasks").unwrap() {
            let _ = conn
                .execute(
                    "CREATE TABLE IF NOT EXISTS tasks (
                      id INTEGER PRIMARY KEY,
                      title TEXT,
                      description TEXT,
                      created INTEGER,
                      due INTEGER,
                      generated_by INTEGER,
                      completed INTEGER
                    );",
                    [],
                )
                .map_err(|err| format!("could not create table: {err}"))?;
        }

        Ok(Self { conn })
    }

    fn add_task(
        &mut self,
        title: String,
        description: Option<String>,
        due: Option<LocalDT>,
    ) -> Result<(), String> {
        let created = chrono::Local::now();
        let _ = self.conn.execute(
            "INSERT INTO tasks (title, description, created, due, completed) VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                title.clone(),
                description.to_owned(),
                created.timestamp(),
                due.map(|t| t.timestamp()),
                Null
            ),
        ).map_err(|err| { format!("could not insert task: {err}") })?;
        Ok(())
    }

    fn import_datetime(x: i64) -> LocalDT {
        chrono::DateTime::from_timestamp(x, 0)
            .unwrap()
            .with_timezone(&Local)
    }

    fn show_tasks(&self, all: bool, verbose: bool) -> Result<(), String> {
        let mut res = self
            .conn
            .prepare("SELECT id, title, description, created, due, completed FROM tasks;")
            .map_err(|err| format!("could not query tasks: {err}"))?;

        let rows = res
            .query_map([], |row| {
                let id: u64 = row.get("ID")?;
                let title: String = row.get("title")?;
                let description: Option<String> = row.get("description")?;

                let created = Self::import_datetime(row.get("created")?);
                let due = row.get::<_, Option<i64>>("due")?.map(Self::import_datetime);
                let completed = row
                    .get::<_, Option<i64>>("completed")?
                    .map(Self::import_datetime);
                Ok((id, title, description, created, due, completed))
            })
            .map_err(|err| format!("Could not query database: {err}"))?;

        for row in rows {
            let (id, title, description, created, due, completed) = match row {
                Ok(row) => row,
                Err(err) => return Err(format!("Error querying database: {err}")),
            };

            if !all && completed.is_some() {
                continue;
            }

            let now = chrono::Local::now();

            let marker = if completed.is_some() { "x" } else { " " };
            let mut heading = format!("- [{marker}] ({id}) {title}").bold();
            if !verbose {
                if completed.is_some() {
                    heading = heading.bright_green();
                } else if let Some(due) = due {
                    if now > due {
                        heading = heading.bright_red();
                    }
                }
            }
            println!("{}", heading);

            if !verbose {
                continue;
            }

            if let Some(completed) = completed {
                let text = format!("completed: {}", completed.format(DATETIME_FMT));
                println!("  {}", text.green());
            }

            let created = format!("  created:   {}", created.format(DATETIME_FMT));
            println!("{}", created);

            if let Some(due) = due {
                let due_repr = format!("  due:       {}", due.format(DATETIME_FMT));
                if now < due {
                    println!("{}", due_repr);
                } else {
                    println!("{}", due_repr.bright_red());
                }
            }

            if let Some(description) = description {
                println!("  {}", description);
            }
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
        let now = Local::now();
        let completed: Option<i64> = self
            .conn
            .query_one("select completed from tasks where id = ?1", (id,), |row| {
                row.get(0)
            })
            .map_err(|err| format!("Could not get task completion status: {err}"))?;
        let completed = completed.map(Self::import_datetime);

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
                (now.timestamp(), id),
            )
            .map_err(|err| format!("Could not mark task {id} as completed: {err}"))?;

        assert_eq!(res, 1);

        Ok(())
    }
}

type LocalDT = chrono::DateTime<Local>;

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

fn parse_date_time(repr: String) -> Result<LocalDT, String> {
    if let Some((date, time)) = repr.split_once(" ") {
        let date = NaiveDate::parse_from_str(date, "%d.%m.%y")
            .map_err(|err| format!("Could not parse date: {err}"))?;
        let time = NaiveTime::parse_from_str(time, "%H:%M")
            .map_err(|err| format!("Could not parse time: {err}"))?;
        let dt = date.and_time(time).and_local_timezone(Local).unwrap();
        Ok(dt)
    } else {
        let date = NaiveDate::parse_from_str(&repr, "%d.%m.%Y")
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
    let args = Args::parse();

    let conn = get_database_connection().unwrap_or_else(|err| {
        eprintln!("Could not get database connection: {err}");
        std::process::exit(1);
    });

    let mut app = App::try_init(conn).unwrap_or_else(|err| {
        eprintln!("ERROR: could not initialize application: {err}");
        std::process::exit(1);
    });

    match args.action {
        Action::Show { all, verbose } => {
            app.show_tasks(all, verbose).unwrap_or_else(|err| {
                eprintln!("Could not show tasks: {err}");
                std::process::exit(1);
            });
        }
        Action::Task {
            title,
            description,
            due,
        } => {
            let due = due.map(parse_date_time).map(|x| {
                x.unwrap_or_else(|err| {
                    eprintln!("Could not parse due date: {}", err);
                    std::process::exit(1);
                })
            });
            app.add_task(title, description, due).unwrap_or_else(|err| {
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

        app.add_task("Test".to_string(), None, None)
            .expect("adding task");

        app.show_tasks(false, true).unwrap();
    }
}
