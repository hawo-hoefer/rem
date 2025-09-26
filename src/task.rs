use colored::Colorize;
use rusqlite::Row;

use crate::{import_datetime, LocalDT, DATETIME_FMT};

pub struct Task {
    pub id: u64,
    pub title: String,
    pub description: Option<String>,

    pub created: LocalDT,
    pub due: Option<LocalDT>,
    pub completed: Option<LocalDT>,
}

impl Task {
    pub fn from_db_row(row: &Row) -> Result<Self, rusqlite::Error> {
        let id: u64 = row.get("ID")?;
        let title: String = row.get("title")?;
        let description: Option<String> = row.get("description")?;

        let created = import_datetime(row.get("created")?);
        let due = row.get::<_, Option<i64>>("due")?.map(import_datetime);
        let completed = row.get::<_, Option<i64>>("completed")?.map(import_datetime);

        Ok(Task {
            id,
            title,
            description,
            created,
            due,
            completed,
        })
    }

    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        all: bool,
        verbose: bool,
        now: LocalDT,
    ) -> std::fmt::Result {
        if !all && self.completed.is_some() {
            return Ok(());
        }

        let marker = if self.completed.is_some() { "x" } else { " " };
        let mut heading = format!(
            "- [{marker}] ({id}) {title}",
            id = self.id,
            title = self.title
        )
        .bold();
        if !verbose {
            if self.completed.is_some() {
                heading = heading.bright_green();
            } else if let Some(due) = self.due {
                if now > due {
                    heading = heading.bright_red();
                }
            }
        }
        writeln!(f, "{}", heading)?;

        if !verbose {
            return Ok(());
        }

        if let Some(completed) = self.completed {
            let text = format!("completed: {}", completed.format(DATETIME_FMT));
            writeln!(f, "  {}", text.green())?;
        }

        let created = format!("  created:   {}", self.created.format(DATETIME_FMT));
        writeln!(f, "{}", created)?;

        if let Some(due) = self.due {
            let due_repr = format!("  due:       {}", due.format(DATETIME_FMT));
            if now < due || self.completed.is_some() {
                writeln!(f, "{}", due_repr)?;
            } else {
                writeln!(f, "{}", due_repr.bright_red())?;
            }
        }

        if let Some(ref description) = self.description {
            writeln!(f, "  {}", description)?;
        }
        Ok(())
    }

    pub fn display<'a>(&'a self, all: bool, verbose: bool, now: LocalDT) -> TaskDisplay<'a> {
        TaskDisplay {
            inner: self,
            all,
            verbose,
            now,
        }
    }
}

pub struct TaskDisplay<'a> {
    inner: &'a Task,
    all: bool,
    verbose: bool,
    now: LocalDT,
}

impl std::fmt::Display for TaskDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f, self.all, self.verbose, self.now)
    }
}
