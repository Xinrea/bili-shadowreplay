use rand::prelude::*;

pub struct UserAgentGenerator {
    rng: ThreadRng,
}

impl Default for UserAgentGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl UserAgentGenerator {
    pub fn new() -> Self {
        Self { rng: rand::rng() }
    }

    /// Generate a user agent
    ///
    /// # Arguments
    ///
    /// * `mobile` - Whether to generate a mobile user agent
    ///
    /// # Returns
    ///
    /// A string representing the user agent
    pub fn generate(&mut self, mobile: bool) -> String {
        if mobile {
            return self.generate_mobile();
        }
        let browser_type = self.rng.random_range(0..4);

        match browser_type {
            0 => self.generate_chrome(),
            1 => self.generate_firefox(),
            2 => self.generate_safari(),
            _ => self.generate_edge(),
        }
    }

    fn generate_mobile(&mut self) -> String {
        let mobile_versions = [
            "120.0.0.0",
            "119.0.0.0",
            "118.0.0.0",
            "117.0.0.0",
            "116.0.0.0",
            "115.0.0.0",
            "114.0.0.0",
        ];
        let mobile_version = mobile_versions.choose(&mut self.rng).unwrap();

        // 随机选择 Android 或 iOS
        if self.rng.random_bool(0.7) {
            // Android User-Agent
            let android_versions = ["13", "12", "11", "10", "9"];
            let android_version = android_versions.choose(&mut self.rng).unwrap();
            let device_models = [
                "SM-G991B",
                "SM-G996B",
                "SM-G998B",
                "SM-A525F",
                "SM-A725F",
                "Pixel 6",
                "Pixel 7",
                "Pixel 8",
                "OnePlus 9",
                "OnePlus 10",
            ];
            let device_model = device_models.choose(&mut self.rng).unwrap();

            format!("Mozilla/5.0 (Linux; Android {android_version}; {device_model}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{mobile_version} Mobile Safari/537.36")
        } else {
            // iOS User-Agent
            let ios_versions = ["17_1", "16_7", "16_6", "15_7", "14_8"];
            let ios_version = ios_versions.choose(&mut self.rng).unwrap();
            let device_types = ["iPhone; CPU iPhone OS", "iPad; CPU OS"];
            let device_type = device_types.choose(&mut self.rng).unwrap();

            format!("Mozilla/5.0 ({device_type} {ios_version} like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.1 Mobile/15E148 Safari/604.1")
        }
    }

    fn generate_chrome(&mut self) -> String {
        let chrome_versions = [
            "120.0.0.0",
            "119.0.0.0",
            "118.0.0.0",
            "117.0.0.0",
            "116.0.0.0",
            "115.0.0.0",
            "114.0.0.0",
        ];
        let webkit_versions = ["537.36", "537.35", "537.34"];

        let os = self.get_random_os();
        let chrome_version = chrome_versions.choose(&mut self.rng).unwrap();
        let webkit_version = webkit_versions.choose(&mut self.rng).unwrap();

        format!(
            "Mozilla/5.0 ({os}) AppleWebKit/{webkit_version} (KHTML, like Gecko) Chrome/{chrome_version} Safari/{webkit_version}"
        )
    }

    fn generate_firefox(&mut self) -> String {
        let firefox_versions = ["121.0", "120.0", "119.0", "118.0", "117.0", "116.0"];

        let os = self.get_random_os_firefox();
        let firefox_version = firefox_versions.choose(&mut self.rng).unwrap();

        format!("Mozilla/5.0 ({os}; rv:{firefox_version}) Gecko/20100101 Firefox/{firefox_version}")
    }

    fn generate_safari(&mut self) -> String {
        let safari_versions = ["17.1", "17.0", "16.6", "16.5", "16.4", "16.3"];
        let webkit_versions = ["605.1.15", "605.1.14", "605.1.13"];

        let safari_version = safari_versions.choose(&mut self.rng).unwrap();
        let webkit_version = webkit_versions.choose(&mut self.rng).unwrap();

        // Safari 只在 macOS 和 iOS 上
        let is_mobile = self.rng.random_bool(0.3);

        if is_mobile {
            let ios_versions = ["17_1", "16_7", "16_6", "15_7"];
            let ios_version = ios_versions.choose(&mut self.rng).unwrap();
            let device = ["iPhone; CPU iPhone OS", "iPad; CPU OS"]
                .choose(&mut self.rng)
                .unwrap();

            format!(
                "Mozilla/5.0 ({device} {ios_version} like Mac OS X) AppleWebKit/{webkit_version} (KHTML, like Gecko) Version/{safari_version} Mobile/15E148 Safari/{webkit_version}"
            )
        } else {
            let macos_versions = ["14_1", "13_6", "12_7"];
            let macos_version = macos_versions.choose(&mut self.rng).unwrap();

            format!(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X {macos_version}) AppleWebKit/{webkit_version} (KHTML, like Gecko) Version/{safari_version} Safari/{webkit_version}"
            )
        }
    }

    fn generate_edge(&mut self) -> String {
        let edge_versions = ["119.0.0.0", "118.0.0.0", "117.0.0.0", "116.0.0.0"];
        let chrome_versions = ["119.0.0.0", "118.0.0.0", "117.0.0.0", "116.0.0.0"];

        let os = self.get_random_os();
        let edge_version = edge_versions.choose(&mut self.rng).unwrap();
        let chrome_version = chrome_versions.choose(&mut self.rng).unwrap();

        format!(
            "Mozilla/5.0 ({os}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{chrome_version} Safari/537.36 Edg/{edge_version}"
        )
    }

    fn get_random_os(&mut self) -> &'static str {
        let os_list = [
            "Windows NT 10.0; Win64; x64",
            "Windows NT 11.0; Win64; x64",
            "Macintosh; Intel Mac OS X 10_15_7",
            "Macintosh; Intel Mac OS X 10_14_6",
            "X11; Linux x86_64",
            "X11; Ubuntu; Linux x86_64",
        ];

        os_list.choose(&mut self.rng).unwrap()
    }

    fn get_random_os_firefox(&mut self) -> &'static str {
        let os_list = [
            "Windows NT 10.0; Win64; x64",
            "Windows NT 11.0; Win64; x64",
            "Macintosh; Intel Mac OS X 10.15",
            "X11; Linux x86_64",
            "X11; Ubuntu; Linux i686",
        ];

        os_list.choose(&mut self.rng).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_user_agents() {
        let mut generator = UserAgentGenerator::new();

        for _ in 0..100 {
            let ua = generator.generate(false);
            assert!(!ua.is_empty());
            assert!(ua.starts_with("Mozilla/5.0"));

            // 验证是否包含常见浏览器标识
            assert!(
                ua.contains("Chrome")
                    || ua.contains("Firefox")
                    || ua.contains("Safari")
                    || ua.contains("Edg")
            );
        }
    }

    #[test]
    fn test_chrome_user_agent_format() {
        let mut generator = UserAgentGenerator::new();
        let ua = generator.generate_chrome();

        assert!(ua.contains("Chrome"));
        assert!(ua.contains("Safari"));
        assert!(ua.contains("AppleWebKit"));
    }

    #[test]
    fn test_mobile_user_agent_format() {
        let mut generator = UserAgentGenerator::new();

        for _ in 0..50 {
            let ua = generator.generate(true);
            assert!(!ua.is_empty());
            assert!(ua.starts_with("Mozilla/5.0"));

            // 验证是否包含移动设备标识
            assert!(ua.contains("Android") || ua.contains("iPhone") || ua.contains("iPad"));

            // 验证是否包含移动浏览器标识
            // Android 包含 Chrome 和 Mobile Safari
            // iOS 包含 Safari
            assert!(ua.contains("Mobile Safari") || ua.contains("Chrome") || ua.contains("Safari"));
        }
    }
}
