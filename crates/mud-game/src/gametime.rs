//! Mud calendar and weather: reset_time/reset_weather, the weather engine,
//! and mud_time_passed/mud_time_to_secs.

use mud_data::rng::CircleRng;
use mud_data::types::{SECS_PER_MUD_DAY, SECS_PER_MUD_HOUR, SECS_PER_MUD_MONTH, SECS_PER_MUD_YEAR};

/// Hardcoded epoch fallback.
pub const DEFAULT_BEGINNING_OF_TIME: i64 = 650336715;

pub const SUN_DARK: i32 = 0;
pub const SUN_RISE: i32 = 1;
pub const SUN_LIGHT: i32 = 2;
pub const SUN_SET: i32 = 3;

pub const SKY_CLOUDLESS: i32 = 0;
pub const SKY_CLOUDY: i32 = 1;
pub const SKY_RAINING: i32 = 2;
pub const SKY_LIGHTNING: i32 = 3;

#[derive(Debug, Clone, Copy, Default)]
pub struct MudTime {
    pub hours: i32,
    pub day: i32,
    pub month: i32,
    pub year: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Weather {
    pub pressure: i32,
    pub change: i32,
    pub sky: i32,
    pub sunlight: i32,
}

pub fn mud_time_passed(t2: i64, t1: i64) -> MudTime {
    let mut secs = t2 - t1;
    let hours = (secs / SECS_PER_MUD_HOUR as i64) % 24;
    secs -= SECS_PER_MUD_HOUR as i64 * hours;
    let day = (secs / SECS_PER_MUD_DAY as i64) % 35;
    secs -= SECS_PER_MUD_DAY as i64 * day;
    let month = (secs / SECS_PER_MUD_MONTH as i64) % 17;
    secs -= SECS_PER_MUD_MONTH as i64 * month;
    let year = secs / SECS_PER_MUD_YEAR as i64;
    MudTime { hours: hours as i32, day: day as i32, month: month as i32, year: year as i32 }
}

/// mud_time_to_secs: re-derive the epoch from `now`.
pub fn mud_time_to_secs(t: &MudTime, now: i64) -> i64 {
    let mut secs = 0i64;
    secs += t.year as i64 * SECS_PER_MUD_YEAR as i64;
    secs += t.month as i64 * SECS_PER_MUD_MONTH as i64;
    secs += t.day as i64 * SECS_PER_MUD_DAY as i64;
    secs += t.hours as i64 * SECS_PER_MUD_HOUR as i64;
    now - secs
}

/// age(ch): mud time since birth + 17 years.
pub fn age(birth: i64, now: i64) -> MudTime {
    let mut t = mud_time_passed(now, birth);
    t.year += 17;
    t
}

/// real_time_passed, reduced to the (days, hours) pair the
/// callers use: hours = (secs/3600)%24, days = secs/86400.
pub fn real_time_passed_hours_days(secs: i64) -> (i64, i64) {
    let hours = (secs / 3600) % 24;
    let days = secs / 86400;
    (days, hours)
}

/// reset_time: derive time_info + sunlight; reset_weather.
/// Consumes RNG for the initial pressure — call order matters at boot.
pub fn reset_time(beginning_of_time: i64, now: i64, rng: &mut CircleRng) -> (MudTime, Weather) {
    let time_info = mud_time_passed(now, beginning_of_time);
    let sunlight = match time_info.hours {
        h if h <= 4 => SUN_DARK,
        5 => SUN_RISE,
        h if h <= 20 => SUN_LIGHT,
        21 => SUN_SET,
        _ => SUN_DARK,
    };
    let mut weather = Weather { pressure: 960, change: 0, sky: 0, sunlight };
    if (7..=12).contains(&time_info.month) {
        weather.pressure += rng.dice(1, 50);
    } else {
        weather.pressure += rng.dice(1, 80);
    }
    weather.sky = if weather.pressure <= 980 {
        SKY_LIGHTNING
    } else if weather.pressure <= 1000 {
        SKY_RAINING
    } else if weather.pressure <= 1020 {
        SKY_CLOUDY
    } else {
        SKY_CLOUDLESS
    };
    (time_info, weather)
}

/// Messages produced by an hourly tick, to broadcast to outdoor players.
pub struct WeatherTick {
    pub messages: Vec<&'static [u8]>,
}

/// another_hour: advance the clock, emit sun messages.
pub fn another_hour(time_info: &mut MudTime, weather: &mut Weather) -> WeatherTick {
    let mut messages = Vec::new();
    time_info.hours += 1;
    match time_info.hours {
        5 => {
            weather.sunlight = SUN_RISE;
            messages.push(b"The sun rises in the east.\r\n" as &[u8]);
        }
        6 => {
            weather.sunlight = SUN_LIGHT;
            messages.push(b"The day has begun.\r\n" as &[u8]);
        }
        21 => {
            weather.sunlight = SUN_SET;
            messages.push(b"The sun slowly disappears in the west.\r\n" as &[u8]);
        }
        22 => {
            weather.sunlight = SUN_DARK;
            messages.push(b"The night has begun.\r\n" as &[u8]);
        }
        _ => {}
    }
    if time_info.hours > 23 {
        time_info.hours -= 24;
        time_info.day += 1;
        if time_info.day > 34 {
            time_info.day = 0;
            time_info.month += 1;
            if time_info.month > 16 {
                time_info.month = 0;
                time_info.year += 1;
            }
        }
    }
    WeatherTick { messages }
}

/// weather_change. The RNG draw order is load-bearing.
pub fn weather_change(time_info: &MudTime, weather: &mut Weather, rng: &mut CircleRng) -> WeatherTick {
    let mut messages = Vec::new();
    let diff = if (9..=16).contains(&time_info.month) {
        if weather.pressure > 985 { -2 } else { 2 }
    } else if weather.pressure > 1015 {
        -2
    } else {
        2
    };
    weather.change += rng.dice(1, 4) * diff + rng.dice(2, 6) - rng.dice(2, 6);
    weather.change = weather.change.clamp(-12, 12);
    weather.pressure += weather.change;
    weather.pressure = weather.pressure.clamp(960, 1040);

    match weather.sky {
        SKY_CLOUDLESS => {
            if weather.pressure < 990 || (weather.pressure < 1010 && rng.dice(1, 4) == 1) {
                messages.push(b"The sky starts to get cloudy.\r\n" as &[u8]);
                weather.sky = SKY_CLOUDY;
            }
        }
        SKY_CLOUDY => {
            if weather.pressure < 970 || (weather.pressure < 990 && rng.dice(1, 4) == 1) {
                messages.push(b"It starts to rain.\r\n" as &[u8]);
                weather.sky = SKY_RAINING;
            } else if weather.pressure > 1030 && rng.dice(1, 4) == 1 {
                messages.push(b"The clouds disappear.\r\n" as &[u8]);
                weather.sky = SKY_CLOUDLESS;
            }
        }
        SKY_RAINING => {
            if weather.pressure < 970 && rng.dice(1, 4) == 1 {
                messages.push(b"Lightning starts to show in the sky.\r\n" as &[u8]);
                weather.sky = SKY_LIGHTNING;
            } else if weather.pressure > 1030 || (weather.pressure > 1010 && rng.dice(1, 4) == 1) {
                messages.push(b"The rain stops.\r\n" as &[u8]);
                weather.sky = SKY_CLOUDY;
            }
        }
        SKY_LIGHTNING => {
            if weather.pressure > 1010 || (weather.pressure > 990 && rng.dice(1, 4) == 1) {
                messages.push(b"The lightning stops.\r\n" as &[u8]);
                weather.sky = SKY_RAINING;
            }
        }
        _ => {
            weather.sky = SKY_CLOUDLESS;
        }
    }
    WeatherTick { messages }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_decomposition_roundtrips() {
        let epoch = DEFAULT_BEGINNING_OF_TIME;
        let now = epoch + 3 * SECS_PER_MUD_YEAR as i64 + 5 * SECS_PER_MUD_MONTH as i64 + 11 * SECS_PER_MUD_DAY as i64 + 7 * 75;
        let t = mud_time_passed(now, epoch);
        assert_eq!((t.year, t.month, t.day, t.hours), (3, 5, 11, 7));
        assert_eq!(mud_time_to_secs(&t, now), epoch);
    }

    #[test]
    fn hour_rollover_rolls_calendar() {
        let mut t = MudTime { hours: 23, day: 34, month: 16, year: 9 };
        let mut w = Weather::default();
        another_hour(&mut t, &mut w);
        assert_eq!((t.hours, t.day, t.month, t.year), (0, 0, 0, 10));
    }
}
