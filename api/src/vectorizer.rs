const MAX_AMOUNT: f64 = 10000.0;
const MAX_INSTALLMENTS: f64 = 12.0;
const AMOUNT_VS_AVG_RATIO: f64 = 10.0;
const MAX_MINUTES: f64 = 1440.0;
const MAX_KM: f64 = 1000.0;
const MAX_TX_COUNT_24H: f64 = 20.0;
const MAX_MERCHANT_AVG_AMOUNT: f64 = 10000.0;
const SCALE: f32 = 10000.0;

fn clamp(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

fn mcc_risk(mcc: &str) -> f64 {
    match mcc {
        "5411" => 0.15,
        "5812" => 0.30,
        "5912" => 0.20,
        "5944" => 0.45,
        "7801" => 0.80,
        "7802" => 0.75,
        "7995" => 0.85,
        "4511" => 0.35,
        "5311" => 0.25,
        "5999" => 0.50,
        _ => 0.5,
    }
}

fn is_leap_year(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(y) => 29,
        2 => 28,
        _ => unreachable!(),
    }
}

fn to_epoch_seconds(dt: (u16, u8, u8, u8, u8, u8)) -> i64 {
    let (y, m, d, hh, mm, ss) = dt;
    let mut total = 0i64;
    for year in 1970..y as i32 {
        total += if is_leap_year(year) { 366 } else { 365 };
    }
    for month in 1..m as u32 {
        total += days_in_month(y as i32, month) as i64;
    }
    total += d as i64 - 1;
    total * 86400 + hh as i64 * 3600 + mm as i64 * 60 + ss as i64
}

fn minutes_between(later: (u16, u8, u8, u8, u8, u8), earlier: (u16, u8, u8, u8, u8, u8)) -> f64 {
    let diff_secs = to_epoch_seconds(later) - to_epoch_seconds(earlier);
    if diff_secs <= 0 {
        0.0
    } else {
        diff_secs as f64 / 60.0
    }
}

fn day_of_week(y: u16, m: u8, d: u8) -> u8 {
    let t = [0u8, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y as i32 - 1 } else { y as i32 };
    let dow = ((y + y / 4 - y / 100 + y / 400 + t[m as usize - 1] as i32 + d as i32) % 7) as u8;
    if dow == 0 { 6 } else { dow - 1 }
}

fn parse_iso_datetime(s: &[u8]) -> Result<(u16, u8, u8, u8, u8, u8), ()> {
    if s.len() < 19 || s[4] != b'-' || s[7] != b'-' || s[10] != b'T' || s[13] != b':' || s[16] != b':' {
        return Err(());
    }
    let year = atoi_u16(&s[0..4])?;
    let month = atoi_u8(&s[5..7])?;
    let day = atoi_u8(&s[8..10])?;
    let hour = atoi_u8(&s[11..13])?;
    let minute = atoi_u8(&s[14..16])?;
    let second = atoi_u8(&s[17..19])?;
    Ok((year, month, day, hour, minute, second))
}

fn atoi_u16(s: &[u8]) -> Result<u16, ()> {
    let mut n: u16 = 0;
    for &b in s {
        if !b.is_ascii_digit() { return Err(()); }
        n = n * 10 + (b - b'0') as u16;
    }
    Ok(n)
}

fn atoi_u8(s: &[u8]) -> Result<u8, ()> {
    let mut n: u8 = 0;
    for &b in s {
        if !b.is_ascii_digit() { return Err(()); }
        n = n * 10 + (b - b'0') as u8;
    }
    Ok(n)
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.data.len() && self.data[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.pos < self.data.len() && self.data[self.pos] == b {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_string_raw(&mut self) -> Result<&'a [u8], ()> {
        if !self.eat(b'"') { return Err(()); }
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != b'"' {
            if self.data[self.pos] == b'\\' { self.pos += 1; }
            self.pos += 1;
        }
        if self.pos >= self.data.len() { return Err(()); }
        let result = &self.data[start..self.pos];
        self.pos += 1;
        Ok(result)
    }

    fn parse_number(&mut self) -> Result<f64, ()> {
        let start = self.pos;
        if self.eat(b'-') {}
        while self.pos < self.data.len() && (self.data[self.pos].is_ascii_digit() || self.data[self.pos] == b'.') {
            self.pos += 1;
        }
        let s = std::str::from_utf8(&self.data[start..self.pos]).map_err(|_| ())?;
        s.parse::<f64>().map_err(|_| ())
    }

    fn parse_int(&mut self) -> Result<u32, ()> {
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        let s = std::str::from_utf8(&self.data[start..self.pos]).map_err(|_| ())?;
        s.parse::<u32>().map_err(|_| ())
    }

    fn parse_bool(&mut self) -> Result<bool, ()> {
        if self.pos + 4 <= self.data.len() && &self.data[self.pos..self.pos + 4] == b"true" {
            self.pos += 4;
            Ok(true)
        } else if self.pos + 5 <= self.data.len() && &self.data[self.pos..self.pos + 5] == b"false" {
            self.pos += 5;
            Ok(false)
        } else {
            Err(())
        }
    }

    fn eat_null(&mut self) -> bool {
        if self.pos + 4 <= self.data.len() && &self.data[self.pos..self.pos + 4] == b"null" {
            self.pos += 4;
            true
        } else {
            false
        }
    }

    fn skip_value(&mut self) {
        self.skip_ws();
        if self.pos >= self.data.len() { return; }
        match self.data[self.pos] {
            b'"' => { let _ = self.parse_string_raw(); }
            b'{' | b'[' => {
                let open = self.data[self.pos];
                let close = if open == b'{' { b'}' } else { b']' };
                self.pos += 1;
                let mut depth = 1;
                while self.pos < self.data.len() && depth > 0 {
                    match self.data[self.pos] {
                        c if c == open => depth += 1,
                        c if c == close => depth -= 1,
                        b'"' => {
                            self.pos += 1;
                            while self.pos < self.data.len() && self.data[self.pos] != b'"' {
                                if self.data[self.pos] == b'\\' { self.pos += 1; }
                                self.pos += 1;
                            }
                        }
                        _ => {}
                    }
                    self.pos += 1;
                }
            }
            b't' | b'T' => { while self.pos < self.data.len() && self.data[self.pos].is_ascii_alphabetic() { self.pos += 1; } }
            b'f' | b'F' => { while self.pos < self.data.len() && self.data[self.pos].is_ascii_alphabetic() { self.pos += 1; } }
            b'n' | b'N' => { while self.pos < self.data.len() && self.data[self.pos].is_ascii_alphabetic() { self.pos += 1; } }
            _ => {
                while self.pos < self.data.len() && (self.data[self.pos].is_ascii_digit() || self.data[self.pos] == b'.' || self.data[self.pos] == b'-' || self.data[self.pos] == b'+' || self.data[self.pos] == b'e' || self.data[self.pos] == b'E') {
                    self.pos += 1;
                }
            }
        }
    }

    fn read_key(&mut self) -> Result<&'a [u8], ()> {
        self.skip_ws();
        if !self.eat(b'"') { return Err(()); }
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != b'"' {
            if self.data[self.pos] == b'\\' { self.pos += 1; }
            self.pos += 1;
        }
        let key = &self.data[start..self.pos];
        if !self.eat(b'"') { return Err(()); }
        self.skip_ws();
        if !self.eat(b':') { return Err(()); }
        self.skip_ws();
        Ok(key)
    }

    fn read_object<F>(&mut self, mut f: F) -> Result<(), ()>
    where
        F: FnMut(&[u8], &mut Cursor) -> Result<(), ()>,
    {
        if !self.eat(b'{') { return Err(()); }
        self.skip_ws();
        if self.eat(b'}') { return Ok(()); }
        loop {
            let key = self.read_key()?;
            f(key, self)?;
            self.skip_ws();
            if self.eat(b'}') { return Ok(()); }
            if !self.eat(b',') { return Err(()); }
            self.skip_ws();
        }
    }

    fn parse_string_array(&mut self) -> Result<Vec<String>, ()> {
        if !self.eat(b'[') { return Err(()); }
        self.skip_ws();
        let mut result = Vec::new();
        if self.eat(b']') { return Ok(result); }
        loop {
            let s = self.parse_string_raw()?;
            result.push(String::from_utf8_lossy(s).to_string());
            self.skip_ws();
            if self.eat(b']') { return Ok(result); }
            if !self.eat(b',') { return Err(()); }
            self.skip_ws();
        }
    }
}

pub fn parse_and_vectorize(body: &[u8]) -> Result<[f32; 14], ()> {
    let mut s = Cursor { data: body, pos: 0 };

    let mut amount = 0.0f64;
    let mut installments = 0u32;
    let mut requested_at = (0u16, 0u8, 0u8, 0u8, 0u8, 0u8);
    let mut customer_avg_amount = 0.0f64;
    let mut customer_tx_count_24h = 0u32;
    let mut known_merchants: Vec<String> = Vec::new();
    let mut merchant_id = String::new();
    let mut merchant_mcc = String::new();
    let mut merchant_avg_amount = 0.0f64;
    let mut terminal_is_online = false;
    let mut terminal_card_present = false;
    let mut terminal_km_from_home = 0.0f64;
    let mut last_tx_minutes = -1.0f64;
    let mut last_tx_km = -1.0f64;

    s.skip_ws();

    s.read_object(|key, s| {
        match key {
            b"id" => { s.skip_value(); }
            b"transaction" => {
                s.read_object(|k, s| {
                    match k {
                        b"amount" => { amount = s.parse_number()?; }
                        b"installments" => { installments = s.parse_int()?; }
                        b"requested_at" => {
                            let ts = s.parse_string_raw()?;
                            requested_at = parse_iso_datetime(ts)?;
                        }
                        _ => { s.skip_value(); }
                    }
                    Ok(())
                })?;
            }
            b"customer" => {
                s.read_object(|k, s| {
                    match k {
                        b"avg_amount" => { customer_avg_amount = s.parse_number()?; }
                        b"tx_count_24h" => { customer_tx_count_24h = s.parse_int()?; }
                        b"known_merchants" => { known_merchants = s.parse_string_array()?; }
                        _ => { s.skip_value(); }
                    }
                    Ok(())
                })?;
            }
            b"merchant" => {
                s.read_object(|k, s| {
                    match k {
                        b"id" => { merchant_id = String::from_utf8_lossy(s.parse_string_raw()?).to_string(); }
                        b"mcc" => { merchant_mcc = String::from_utf8_lossy(s.parse_string_raw()?).to_string(); }
                        b"avg_amount" => { merchant_avg_amount = s.parse_number()?; }
                        _ => { s.skip_value(); }
                    }
                    Ok(())
                })?;
            }
            b"terminal" => {
                s.read_object(|k, s| {
                    match k {
                        b"is_online" => { terminal_is_online = s.parse_bool()?; }
                        b"card_present" => { terminal_card_present = s.parse_bool()?; }
                        b"km_from_home" => { terminal_km_from_home = s.parse_number()?; }
                        _ => { s.skip_value(); }
                    }
                    Ok(())
                })?;
            }
            b"last_transaction" => {
                if s.eat_null() {
                } else {
                    let mut lt_timestamp = (0u16, 0u8, 0u8, 0u8, 0u8, 0u8);
                    let mut lt_km = 0.0f64;
                    s.read_object(|k, s| {
                        match k {
                            b"timestamp" => {
                                let ts = s.parse_string_raw()?;
                                lt_timestamp = parse_iso_datetime(ts)?;
                            }
                            b"km_from_current" => { lt_km = s.parse_number()?; }
                            _ => { s.skip_value(); }
                        }
                        Ok(())
                    })?;
                    last_tx_minutes = minutes_between(requested_at, lt_timestamp);
                    last_tx_km = lt_km;
                }
            }
            _ => { s.skip_value(); }
        }
        Ok(())
    })?;

    if requested_at.1 == 0 { return Err(()); }
    let unknown_merchant = if known_merchants.contains(&merchant_id) { 0.0 } else { 1.0 };

    let v0 = clamp(amount / MAX_AMOUNT);
    let v1 = clamp(installments as f64 / MAX_INSTALLMENTS);
    let v2 = clamp((amount / customer_avg_amount.max(0.000001)) / AMOUNT_VS_AVG_RATIO);
    let v3 = requested_at.3 as f64 / 23.0;
    let v4 = day_of_week(requested_at.0, requested_at.1, requested_at.2) as f64 / 6.0;
    let v5 = if last_tx_minutes < 0.0 { -1.0 } else { clamp(last_tx_minutes / MAX_MINUTES) };
    let v6 = if last_tx_km < 0.0 { -1.0 } else { clamp(last_tx_km / MAX_KM) };
    let v7 = clamp(terminal_km_from_home / MAX_KM);
    let v8 = clamp(customer_tx_count_24h as f64 / MAX_TX_COUNT_24H);
    let v9 = if terminal_is_online { 1.0 } else { 0.0 };
    let v10 = if terminal_card_present { 1.0 } else { 0.0 };
    let v11 = unknown_merchant;
    let v12 = mcc_risk(&merchant_mcc);
    let v13 = clamp(merchant_avg_amount / MAX_MERCHANT_AVG_AMOUNT);

    Ok([
        v0 as f32, v1 as f32, v2 as f32, v3 as f32, v4 as f32,
        v5 as f32, v6 as f32, v7 as f32, v8 as f32, v9 as f32,
        v10 as f32, v11 as f32, v12 as f32, v13 as f32,
    ])
}

pub fn quantize(vector: &[f32; 14]) -> [i16; 14] {
    std::array::from_fn(|i| (vector[i] * SCALE).round() as i16)
}
