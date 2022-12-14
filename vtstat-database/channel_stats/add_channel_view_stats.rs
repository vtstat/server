use chrono::{DateTime, Utc};
use sqlx::{postgres::PgQueryResult, PgPool, Postgres, QueryBuilder, Result};

pub struct AddChannelViewStatsQuery {
    pub time: DateTime<Utc>,
    pub rows: Vec<AddChannelViewStatsRow>,
}

pub struct AddChannelViewStatsRow {
    pub channel_id: i32,
    pub count: i32,
}

impl AddChannelViewStatsQuery {
    pub async fn execute(self, pool: &PgPool) -> Result<PgQueryResult> {
        let mut query_builder: QueryBuilder<Postgres> =
            QueryBuilder::new("INSERT INTO channel_view_stats AS s (channel_id, time, count) ");

        query_builder.push_values(self.rows.iter(), |mut b, row| {
            b.push_bind(row.channel_id)
                .push_bind(self.time)
                .push_bind(row.count);
        });

        query_builder.push(
            "ON CONFLICT (channel_id, time) DO UPDATE \
            SET count = GREATEST(excluded.count, s.count)",
        );

        let query = query_builder.build().execute(pool);

        crate::otel::instrument("INSERT", "channel_view_stats", query).await
    }
}

#[cfg(test)]
#[sqlx::test(fixtures("channels"))]
async fn test(pool: PgPool) -> Result<()> {
    use super::ChannelsViewStats;
    use chrono::NaiveDateTime;

    let time = DateTime::from_utc(NaiveDateTime::from_timestamp(9000, 0), Utc);

    let result = AddChannelViewStatsQuery {
        time,
        rows: vec![
            AddChannelViewStatsRow {
                channel_id: 1,
                count: 20,
            },
            AddChannelViewStatsRow {
                channel_id: 2,
                count: 10,
            },
            AddChannelViewStatsRow {
                channel_id: 3,
                count: 20,
            },
        ],
    }
    .execute(&pool)
    .await?;

    assert_eq!(result.rows_affected(), 3);
    let stats = sqlx::query_as!(
        ChannelsViewStats,
        "SELECT time, count FROM channel_view_stats"
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(stats.len(), 3);

    let result = AddChannelViewStatsQuery {
        time,
        rows: vec![
            AddChannelViewStatsRow {
                channel_id: 2,
                count: 20,
            },
            AddChannelViewStatsRow {
                channel_id: 3,
                count: 10,
            },
            AddChannelViewStatsRow {
                channel_id: 4,
                count: 20,
            },
        ],
    }
    .execute(&pool)
    .await?;

    assert_eq!(result.rows_affected(), 3);
    let stats = sqlx::query_as!(
        ChannelsViewStats,
        "SELECT time, count FROM channel_view_stats"
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(stats.len(), 4);
    assert!(stats.iter().all(|s| s.count == 20));
    assert!(stats.iter().all(|s| s.time == time));

    Ok(())
}
