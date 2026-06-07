use std::net::Ipv4Addr;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Lease {
    pub ip_address: Ipv4Addr,
    pub hostname: Option<String>,
    pub state: Option<i64>,
    pub subnet_id: Option<u32>,
    pub valid_lft: Option<u64>,
    pub cltt: Option<u64>,
    pub expire: Option<u64>,
}

impl Lease {
    pub fn expires_at(&self) -> Option<u64> {
        if let Some(expire) = self.expire {
            return Some(expire);
        }

        let cltt = self.cltt?;
        let valid_lft = self.valid_lft?;
        cltt.checked_add(valid_lft)
    }

    pub fn ordering_timestamp(&self) -> u64 {
        self.cltt.or(self.expire).unwrap_or_default()
    }

    pub fn is_active(&self, now_epoch_seconds: u64) -> bool {
        self.state == Some(0)
            && self.valid_lft.unwrap_or_default() > 0
            && self
                .expires_at()
                .is_some_and(|expires_at| expires_at > now_epoch_seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_active_lease() {
        let lease = Lease {
            ip_address: Ipv4Addr::new(192, 0, 2, 10),
            hostname: Some("host01".to_string()),
            state: Some(0),
            subnet_id: Some(10),
            valid_lft: Some(300),
            cltt: Some(100),
            expire: None,
        };

        assert!(lease.is_active(150));
        assert!(!lease.is_active(401));
    }
}
