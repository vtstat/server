use futures::{stream, TryStreamExt};
use vtstat_database::{
    channels::ListChannelsQuery,
    streams::{ListYouTubeStreamsQuery, StreamStatus as StreamStatus_, UpsertYouTubeStreamQuery},
    PgPool,
};
use vtstat_request::{RequestHub, StreamStatus};

use super::JobResult;
use crate::timer::{timer, Calendar};

pub async fn execute(pool: PgPool, hub: RequestHub) -> anyhow::Result<JobResult> {
    let (current_run, next_run) = timer(Calendar::Hourly);

    let now_str = current_run.to_string();

    let youtube_channels = ListChannelsQuery {
        platform: "youtube",
    }
    .execute(&pool)
    .await?;

    let feeds = stream::unfold(youtube_channels.iter(), |mut iter| async {
        let channel = iter.next()?;
        let res = hub.fetch_rss_feed(&channel.platform_id, &now_str).await;
        Some((res, iter))
    })
    .try_collect::<Vec<String>>()
    .await?;

    let video_ids = feeds
        .iter()
        .filter_map(|feed| find_video_id(feed))
        .collect::<Vec<_>>();

    let existed = ListYouTubeStreamsQuery {
        platform_ids: &video_ids,
        limit: None,
        ..Default::default()
    }
    .execute(&pool)
    .await?;

    let missing = video_ids
        .into_iter()
        .filter(|id| existed.iter().all(|stream| &stream.platform_id != id))
        .collect::<Vec<_>>();

    if missing.is_empty() {
        return Ok(JobResult::Next {
            run: next_run,
            continuation: None,
        });
    }

    tracing::debug!("Missing video ids: {:?}", missing);

    let streams = hub.youtube_streams(&missing).await?;

    if streams.is_empty() {
        tracing::debug!("Stream not found, ids={:?}", missing);
        return Ok(JobResult::Next {
            run: next_run,
            continuation: None,
        });
    }

    for stream in streams {
        let channel = youtube_channels
            .iter()
            .find(|ch| ch.platform_id == stream.channel_id);

        let Some(channel) = channel else {
            continue;
        };

        let thumbnail_url = hub.upload_thumbnail(&stream.id).await;

        UpsertYouTubeStreamQuery {
            platform_stream_id: &stream.id,
            channel_id: channel.channel_id,
            title: &stream.title,
            status: match stream.status {
                StreamStatus::Ended => StreamStatus_::Ended,
                StreamStatus::Live => StreamStatus_::Live,
                StreamStatus::Scheduled => StreamStatus_::Scheduled,
            },
            thumbnail_url,
            schedule_time: stream.schedule_time,
            start_time: stream.start_time,
            end_time: stream.end_time,
        }
        .execute(&pool)
        .await?;
    }

    Ok(JobResult::Next {
        run: next_run,
        continuation: None,
    })
}

// TODO: add unit tests

fn find_video_id(feed: &str) -> Option<String> {
    // <yt:videoId>XXXXXXXXXXX</yt:videoId>
    Some(
        feed.lines()
            .nth(14)?
            .trim()
            .strip_prefix("<yt:videoId>")?
            .strip_suffix("</yt:videoId>")?
            .into(),
    )
}
