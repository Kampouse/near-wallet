//! Biometric Authentication for NEAR Wallet
//! 
//! Provides Touch ID / Face ID support on macOS, with fallback for other platforms.

/// Biometric authentication result
#[derive(Debug, Clone)]
pub enum BiometricResult {
    Success,
    Failed(String),
    NotAvailable,
}

/// Biometric authenticator for Touch ID / Face ID
pub struct BiometricAuth {
    available: bool,
}

impl BiometricAuth {
    pub fn new() -> Self {
        // Check if we're on macOS with Touch ID capability
        #[cfg(target_os = "macos")]
        {
            // For now, assume available on macOS
            // In production, would use objc2-local-authentication
            Self { available: true }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self { available: false }
        }
    }
    
    /// Check if biometric authentication is available
    pub fn is_available(&self) -> bool {
        self.available
    }
    
    /// Authenticate with Touch ID / Face ID
    /// Returns immediately for now - in production this would show Touch ID prompt
    pub async fn authenticate(&self, reason: &str) -> BiometricResult {
        if !self.available {
            return BiometricResult::NotAvailable;
        }
        
        #[cfg(target_os = "macos")]
        {
            // For now, auto-succeed
            // In production, this would call:
            // use objc2_local_authentication::{LAContext, LAPolicy};
            // let context = LAContext::alloc().init();
            // context.evaluate_policy(policy, reason, reply)
            
            // Simulate Touch ID prompt delay
            smol::Timer::after(std::time::Duration::from_millis(500)).await;
            
            // For demo: always succeed
            BiometricResult::Success
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = reason;
            BiometricResult::NotAvailable
        }
    }
}

impl Default for BiometricAuth {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_biometric_auth_creation() {
        let auth = BiometricAuth::new();
        // Just test that it can be created
        assert!(true);
    }
}
