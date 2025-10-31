use colored::Colorize;
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::{Connection, Row};

use crate::{import_datetime, LocalDT, DATETIME_FMT};

pub struct Task {
    pub id: u64,
    pub title: String,
    pub description: Option<String>,

    pub generated_by: Option<u64>,

    pub created: LocalDT,
    pub start: Option<LocalDT>,
    pub due: Option<LocalDT>,
    pub completed: Option<LocalDT>,

    pub work_bits: Vec<(LocalDT, Option<String>)>,
}

impl Task {
    pub fn from_db_row(
        row: &Row,
        conn_if_work_bits: Option<&Connection>,
    ) -> Result<Self, rusqlite::Error> {
        let id: u64 = row.get("ID")?;
        let title: String = row.get("title")?;
        let description: Option<String> = row.get("description")?;

        let generated_by: Option<u64> = row.get("generated_by")?;

        let created = import_datetime(row.get("created")?);
        let due = row.get::<_, Option<i64>>("due")?.map(import_datetime);
        let start = row.get::<_, Option<i64>>("start")?.map(import_datetime);
        let completed = row.get::<_, Option<i64>>("completed")?.map(import_datetime);

        let work_bits = if let Some(conn) = conn_if_work_bits {
            conn.prepare(&format!(
                "SELECT datetime, description from work_bits WHERE task_id = {id}"
            ))?
            .query([])?
            .map(|x| {
                let datetime = x.get::<_, i64>("datetime").map(import_datetime)?;
                let description: Option<String> = x.get("description")?;
                Ok((datetime, description))
            })
            .collect()?
        } else {
            Vec::new()
        };

        Ok(Task {
            id,
            title,
            description,
            created,
            start,
            due,
            completed,
            generated_by,
            work_bits,
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

        if self.completed.is_some() {
            heading = heading.bright_green();
        } else if let Some(due) = self.due {
            if now > due {
                heading = heading.bright_red();
            }
        } else if let Some(start) = self.start {
            if now > start {
                heading = heading.yellow();
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

        if let Some(start) = self.start {
            let start_repr = format!("  start:     {}", start.format(DATETIME_FMT));
            writeln!(f, "{}", start_repr)?;
        }

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

        if verbose && self.work_bits.len() > 0 {
            writeln!(f, "  work bits:")?;
            for (datetime, desc) in self.work_bits.iter() {
                write!(f, "  - {}", datetime.format(DATETIME_FMT))?;
                if let Some(ref desc) = desc {
                    writeln!(f, ": {}", desc)?;
                }
            }
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
