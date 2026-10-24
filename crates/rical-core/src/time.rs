use chrono::{NaiveTime, Utc};
use icalendar::DatePerhapsTime;

pub fn date_perhaps_time_to_datetime_utc(dpt: DatePerhapsTime) -> chrono::DateTime<Utc> {
    match dpt {
        icalendar::DatePerhapsTime::DateTime(calendar_date_time) => calendar_date_time
            .try_into_utc()
            .unwrap_or_else(|| match calendar_date_time {
                icalendar::CalendarDateTime::Floating(naive_date_time) => naive_date_time.and_utc(),
                icalendar::CalendarDateTime::Utc(..) => todo!(),
                icalendar::CalendarDateTime::WithTimezone { .. } => todo!(),
            }),
        icalendar::DatePerhapsTime::Date(naive_date) => naive_date
            .and_time(NaiveTime::from_hms_opt(0, 0, 0).unwrap())
            .and_utc(),
    }
}
