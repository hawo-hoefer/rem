use chrono::TimeDelta;
use colored::Colorize;
use rusqlite::Row;

use crate::{import_datetime, LocalDT, DATETIME_FMT};

pub struct Reminder {
    pub id: u64,
    pub title: String,
    pub description: Option<String>,

    pub created: LocalDT,
    pub first_due: LocalDT,
    pub period: TimeDelta,

    pub until: Option<LocalDT>,
}

impl Reminder {
    pub fn from_db_row(row: &Row<'_>) -> Result<Self, rusqlite::Error> {
        let id: u64 = row.get("id")?;
        let title: String = row.get("title")?;
        let description: Option<String> = row.get("description")?;

        let created = import_datetime(row.get("created")?);
        let first_due = import_datetime(row.get::<_, i64>("first_due")?);
        let period =
            TimeDelta::new(row.get::<_, i64>("period")?, 0).expect("duration is in bounds");

        let until = row.get::<_, Option<i64>>("until")?.map(import_datetime);

        Ok(Self {
            id,
            title,
            description,
            created,
            first_due,
            period,
            until,
        })
    }

    pub fn is_active(&self, now: LocalDT) -> bool {
        self.until.map(|until| now < until).unwrap_or(true)
    }

    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        all: bool,
        verbose: bool,
        now: LocalDT,
    ) -> std::fmt::Result {
        let active = self.is_active(now);
        if !all && !active {
            return Ok(());
        }

        let marker = if !active { "x" } else { " " };
        let mut heading = format!(
            "- [{marker}] ({id}) {title}",
            id = self.id,
            title = self.title
        )
        .bold();
        if !verbose {
            if !active {
                heading = heading.green();
            }
        }
        writeln!(f, "{heading}")?;
        writeln!(f, "  created:   {}", self.created.format(DATETIME_FMT))?;
        writeln!(f, "  first due: {}", self.first_due.format(DATETIME_FMT))?;
        if let Some(until) = self.until {
            writeln!(f, "  until:     {}", until.format(DATETIME_FMT))?;
        }
        let mut next_due = self.first_due;
        while next_due < now {
            next_due += self.period;
        }
        writeln!(f, "  next due:  {}", next_due.format(DATETIME_FMT))?;

        if let Some(ref description) = self.description {
            writeln!(f, "  {description}")?;
        }
        Ok(())
    }

    pub fn display<'a>(&'a self, all: bool, verbose: bool, now: LocalDT) -> ReminderDisplay<'a> {
        ReminderDisplay {
            inner: self,
            all,
            verbose,
            now,
        }
    }
}

pub struct ReminderDisplay<'a> {
    inner: &'a Reminder,
    all: bool,
    verbose: bool,
    now: LocalDT,
}

impl std::fmt::Display for ReminderDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f, self.all, self.verbose, self.now)
    }
}
