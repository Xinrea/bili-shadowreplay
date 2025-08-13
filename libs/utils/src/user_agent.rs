use rand::Rng;

/// Generate a random user agent that matches the rules of the browser
pub fn generate_random_user_agent() -> String {
    let mut rng = rand::thread_rng();

    // Random operating system
    let os = match rng.gen_range(0..4) {
        0 => "Windows NT 10.0; Win64; x64",
        1 => "Macintosh; Intel Mac OS X 10_15_7",
        2 => "X11; Linux x86_64",
        3 => "X11; Ubuntu; Linux x86_64",
        _ => "Windows NT 10.0; Win64; x64",
    };

    // Random Chrome version (major versions from 90 to 120)
    let chrome_major = rng.gen_range(90..121);
    let chrome_minor = rng.gen_range(0..100);
    let chrome_patch = rng.gen_range(0..100);
    let chrome_version = format!("{}.{}.{}.0", chrome_major, chrome_minor, chrome_patch);

    // Random WebKit version (usually 537.36 for Chrome)
    let webkit_version = "537.36";

    // Random Safari version (usually matches Chrome major version)
    let safari_version = format!("{}.0.0.0", chrome_major);

    format!(
        "Mozilla/5.0 ({}) AppleWebKit/{} (KHTML, like Gecko) Chrome/{} Safari/{}",
        os, webkit_version, chrome_version, safari_version
    )
}

/// Generate a random user agent for a specific platform
pub fn generate_platform_user_agent(platform: &str) -> String {
    let mut rng = rand::thread_rng();

    let (os, webkit_version) = match platform.to_lowercase().as_str() {
        "windows" => ("Windows NT 10.0; Win64; x64", "537.36"),
        "macos" => ("Macintosh; Intel Mac OS X 10_15_7", "537.36"),
        "linux" => ("X11; Linux x86_64", "537.36"),
        "ubuntu" => ("X11; Ubuntu; Linux x86_64", "537.36"),
        "android" => ("Linux; Android 11; SM-G991B", "537.36"),
        "ios" => ("iPhone; CPU iPhone OS 15_0 like Mac OS X", "605.1.15"),
        _ => ("Windows NT 10.0; Win64; x64", "537.36"),
    };

    let chrome_major = rng.gen_range(90..121);
    let chrome_minor = rng.gen_range(0..100);
    let chrome_patch = rng.gen_range(0..100);
    let chrome_version = format!("{}.{}.{}.0", chrome_major, chrome_minor, chrome_patch);

    let safari_version = format!("{}.0.0.0", chrome_major);

    format!(
        "Mozilla/5.0 ({}) AppleWebKit/{} (KHTML, like Gecko) Chrome/{} Safari/{}",
        os, webkit_version, chrome_version, safari_version
    )
}

/// Generate a random mobile user agent
pub fn generate_mobile_user_agent() -> String {
    let mut rng = rand::thread_rng();

    let device = match rng.gen_range(0..3) {
        0 => "Linux; Android 11; SM-G991B",
        1 => "iPhone; CPU iPhone OS 15_0 like Mac OS X",
        2 => "iPad; CPU OS 15_0 like Mac OS X",
        _ => "Linux; Android 11; SM-G991B",
    };

    let chrome_major = rng.gen_range(90..121);
    let chrome_minor = rng.gen_range(0..100);
    let chrome_patch = rng.gen_range(0..100);
    let chrome_version = format!("{}.{}.{}.0", chrome_major, chrome_minor, chrome_patch);

    format!(
        "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{} Mobile Safari/537.36",
        device, chrome_version
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_random_user_agent() {
        let ua = generate_random_user_agent();
        assert!(ua.contains("Mozilla/5.0"));
        assert!(ua.contains("AppleWebKit"));
        assert!(ua.contains("Chrome"));
        assert!(ua.contains("Safari"));
    }

    #[test]
    fn test_generate_platform_user_agent() {
        let ua = generate_platform_user_agent("windows");
        assert!(ua.contains("Windows NT 10.0; Win64; x64"));

        let ua = generate_platform_user_agent("macos");
        assert!(ua.contains("Macintosh; Intel Mac OS X"));
    }

    #[test]
    fn test_generate_mobile_user_agent() {
        let ua = generate_mobile_user_agent();
        assert!(ua.contains("Mozilla/5.0"));
        assert!(ua.contains("Mobile Safari"));
    }
}
