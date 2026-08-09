use super::AppState;
use crate::queries::booking::activity_queries::{self, RecentCheckIn, RecentCheckOut};
use crate::{models::*, queries::booking::revenue_queries};
use tauri::State;

// ─── A3: Get Analytics ───

#[tauri::command]
pub async fn get_analytics(
    state: State<'_, AppState>,
    period: String,
) -> Result<AnalyticsData, String> {
    let now = chrono::Local::now();
    let days = match period.as_str() {
        "30d" => 30_i64,
        "90d" => 90_i64,
        _ => 7_i64,
    };
    let from = (now - chrono::Duration::days(days))
        .format("%Y-%m-%d")
        .to_string();
    let to = now.format("%Y-%m-%d").to_string();

    revenue_queries::load_analytics(&state.db, &from, &to, days)
        .await
        .map_err(|e| e.to_string())
}

// ─── A4: Get Recent Activity (Dashboard) ───

#[tauri::command]
pub async fn get_recent_activity(
    state: State<'_, AppState>,
    limit: i32,
) -> Result<Vec<ActivityItem>, String> {
    let check_ins = activity_queries::load_recent_check_ins(&state.db, limit)
        .await
        .map_err(|e| e.to_string())?;
    let check_outs = activity_queries::load_recent_check_outs(&state.db, limit)
        .await
        .map_err(|e| e.to_string())?;

    Ok(build_activity_feed(check_ins, check_outs, limit))
}

/// Turns the two activity reads into the dashboard feed.
///
/// Pure on purpose: everything below here is presentation — the icon, the
/// Tailwind class, and the Vietnamese status label the dashboard renders — so
/// it can be tested without a database.
fn build_activity_feed(
    check_ins: Vec<RecentCheckIn>,
    check_outs: Vec<RecentCheckOut>,
    limit: i32,
) -> Vec<ActivityItem> {
    let mut activities: Vec<ActivityItem> = Vec::new();

    for row in check_ins {
        activities.push(ActivityItem {
            icon: "🟢".to_string(),
            text: format!("Check-in {} → {}", row.guest_name, row.room_id),
            time: extract_time(&row.check_in_at),
            color: "bg-emerald-50".to_string(),
            kind: "check_in".to_string(),
            room_id: Some(row.room_id),
            guest_name: Some(row.guest_name),
            occurred_at: row.check_in_at,
            status_label: "Đã check-in".to_string(),
        });
    }

    for row in check_outs {
        activities.push(ActivityItem {
            icon: "🔴".to_string(),
            text: format!("Check-out {} — Room {}", row.guest_name, row.room_id),
            time: extract_time(&row.actual_checkout),
            color: "bg-red-50".to_string(),
            kind: "check_out".to_string(),
            room_id: Some(row.room_id),
            guest_name: Some(row.guest_name),
            occurred_at: row.actual_checkout,
            status_label: "Đã check-out".to_string(),
        });
    }

    // Sort by full timestamp descending to keep cross-day ordering stable.
    activities.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at));
    activities.truncate(limit.max(0) as usize);

    activities
}

fn extract_time(datetime_str: &str) -> String {
    // Extract HH:MM from ISO datetime or RFC3339
    if let Some(t_pos) = datetime_str.find('T') {
        let time_part = &datetime_str[t_pos + 1..];
        if time_part.len() >= 5 {
            return time_part[..5].to_string();
        }
    }
    // Fallback: try space separator
    if let Some(parts) = datetime_str.split(' ').nth(1) {
        if parts.len() >= 5 {
            return parts[..5].to_string();
        }
    }
    datetime_str.to_string()
}

#[cfg(test)]
mod tests {
    use super::{build_activity_feed, extract_time};
    use crate::queries::booking::activity_queries::{RecentCheckIn, RecentCheckOut};

    fn check_in(room_id: &str, guest: &str, at: &str) -> RecentCheckIn {
        RecentCheckIn {
            room_id: room_id.to_string(),
            guest_name: guest.to_string(),
            check_in_at: at.to_string(),
        }
    }

    fn check_out(room_id: &str, guest: &str, at: &str) -> RecentCheckOut {
        RecentCheckOut {
            room_id: room_id.to_string(),
            guest_name: guest.to_string(),
            actual_checkout: at.to_string(),
        }
    }

    #[test]
    fn the_two_sources_interleave_newest_first() {
        let feed = build_activity_feed(
            vec![check_in("101", "An", "2026-04-10T14:00:00+07:00")],
            vec![check_out("102", "Binh", "2026-04-10T12:00:00+07:00")],
            10,
        );

        let kinds: Vec<&str> = feed.iter().map(|item| item.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["check_in", "check_out"],
            "the feed is ordered by the full timestamp, not by source"
        );
    }

    #[test]
    fn the_limit_applies_to_the_merged_feed_not_to_each_source() {
        let feed = build_activity_feed(
            vec![
                check_in("101", "An", "2026-04-10T09:00:00+07:00"),
                check_in("102", "Binh", "2026-04-10T08:00:00+07:00"),
            ],
            vec![check_out("103", "Cuong", "2026-04-10T10:00:00+07:00")],
            2,
        );

        assert_eq!(feed.len(), 2);
        assert_eq!(feed[0].room_id.as_deref(), Some("103"));
        assert_eq!(feed[1].room_id.as_deref(), Some("101"));
    }

    #[test]
    fn a_check_in_carries_its_presentation() {
        let feed = build_activity_feed(
            vec![check_in("101", "An", "2026-04-10T14:30:00+07:00")],
            vec![],
            10,
        );

        let item = &feed[0];
        assert_eq!(item.icon, "🟢");
        assert_eq!(item.text, "Check-in An → 101");
        assert_eq!(item.time, "14:30");
        assert_eq!(item.color, "bg-emerald-50");
        assert_eq!(item.status_label, "Đã check-in");
        assert_eq!(item.guest_name.as_deref(), Some("An"));
    }

    #[test]
    fn a_check_out_carries_its_presentation() {
        let feed = build_activity_feed(
            vec![],
            vec![check_out("102", "Binh", "2026-04-10T11:05:00+07:00")],
            10,
        );

        let item = &feed[0];
        assert_eq!(item.icon, "🔴");
        assert_eq!(item.text, "Check-out Binh — Room 102");
        assert_eq!(item.time, "11:05");
        assert_eq!(item.color, "bg-red-50");
        assert_eq!(item.status_label, "Đã check-out");
    }

    #[test]
    fn ties_keep_check_in_before_check_out() {
        let same = "2026-04-10T10:00:00+07:00";
        let feed = build_activity_feed(
            vec![check_in("101", "An", same)],
            vec![check_out("102", "Binh", same)],
            10,
        );

        let kinds: Vec<&str> = feed.iter().map(|item| item.kind.as_str()).collect();
        assert_eq!(kinds, vec!["check_in", "check_out"]);
    }

    #[test]
    fn an_empty_feed_survives_a_zero_or_negative_limit() {
        assert!(build_activity_feed(vec![], vec![], 0).is_empty());
        assert!(build_activity_feed(
            vec![check_in("101", "An", "2026-04-10T10:00:00+07:00")],
            vec![],
            -1,
        )
        .is_empty());
    }

    #[test]
    fn extract_time_reads_hh_mm_from_every_shape_it_meets() {
        assert_eq!(extract_time("2026-04-10T14:30:00+07:00"), "14:30");
        assert_eq!(extract_time("2026-04-10 14:30:00"), "14:30");
        assert_eq!(
            extract_time("2026-04-10"),
            "2026-04-10",
            "no time component falls back to the raw string"
        );
        assert_eq!(
            extract_time("2026-04-10T14"),
            "2026-04-10T14",
            "a truncated time component falls back rather than panicking"
        );
        assert_eq!(extract_time(""), "");
    }
}
