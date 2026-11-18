// Keep API dates, wall times and UTC timestamps as strings.
time::serde::format_description!(pub(crate) calendar_date, Date, "[year]-[month]-[day]");

pub(crate) mod wall_time {
    use serde::{Deserialize, Serializer, de::Error};
    use time::{Time, macros::format_description};

    pub(crate) fn serialize<S: Serializer>(value: &Time, serializer: S) -> Result<S::Ok, S::Error> {
        let nanos = value.nanosecond();
        let mut text = format!(
            "{:02}:{:02}:{:02}",
            value.hour(),
            value.minute(),
            value.second()
        );
        if nanos != 0 {
            let fraction = if nanos.is_multiple_of(1_000_000) {
                format!("{:03}", nanos / 1_000_000)
            } else if nanos.is_multiple_of(1_000) {
                format!("{:06}", nanos / 1_000)
            } else {
                format!("{nanos:09}")
            };
            text.push('.');
            text.push_str(&fraction);
        }
        serializer.serialize_str(&text)
    }

    pub(crate) fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Time, D::Error> {
        let value = String::deserialize(deserializer)?;
        Time::parse(
            value.trim(),
            format_description!("[hour padding:none]:[minute padding:none][optional [:[second padding:none][optional [.[subsecond]]]]]"),
        )
        .map_err(D::Error::custom)
    }

    pub(crate) mod option {
        use serde::{Deserialize, Deserializer, Serialize, Serializer};
        use time::Time;

        #[derive(Deserialize, Serialize)]
        struct Formatted(#[serde(with = "super")] Time);

        pub(crate) fn serialize<S: Serializer>(
            value: &Option<Time>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            value.map(Formatted).serialize(serializer)
        }

        pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<Time>, D::Error> {
            Option::<Formatted>::deserialize(deserializer)
                .map(|value| value.map(|formatted| formatted.0))
        }
    }
}

pub(crate) mod utc_timestamp {
    use serde::{Deserializer, Serializer};
    use time::{OffsetDateTime, UtcOffset, serde::rfc3339};

    pub(crate) fn serialize<S: Serializer>(
        value: &OffsetDateTime,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        rfc3339::serialize(&value.to_offset(UtcOffset::UTC), serializer)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<OffsetDateTime, D::Error> {
        rfc3339::deserialize(deserializer).map(|value| value.to_offset(UtcOffset::UTC))
    }

    pub(crate) mod option {
        use super::*;

        pub(crate) fn serialize<S: Serializer>(
            value: &Option<OffsetDateTime>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            rfc3339::option::serialize(
                &value.map(|value| value.to_offset(UtcOffset::UTC)),
                serializer,
            )
        }

        pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<OffsetDateTime>, D::Error> {
            rfc3339::option::deserialize(deserializer)
                .map(|value| value.map(|instant| instant.to_offset(UtcOffset::UTC)))
        }
    }
}
