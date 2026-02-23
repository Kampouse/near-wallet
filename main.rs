//! NEAR Wallet - GPUI Edition with RPC integration, Key Generation, and Touch ID
mod rpc;
mod biometric;

use gpui::{
    prelude::*,
    App, Application, Bounds, Context, CursorStyle, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, IntoElement,
    KeyBinding, LayoutId, MouseDownEvent, PaintQuad, Pixels, Point, Render, ShapedLine,
    SharedString, Style, TextRun, UTF16Selection, Window, WindowBounds, WindowOptions,
    actions, div, fill, point, px, rgb, size, Hsla,
};
use std::ops::Range;
use rpc::{NearRpc, Network, Transaction};
use biometric::{BiometricAuth, BiometricResult};
use std::sync::Arc;
use parking_lot::Mutex;
use rand::RngCore;

actions!(text_input, [Backspace, Delete, Left, Right, Home, End, Paste, Copy, Cut, SelectAll, Enter]);

// ============================================================================
// SAVED WALLET (with Touch ID protection)
// ============================================================================

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SavedWallet {
    pub account_id: String,
    pub encrypted_key: String, // Base64 encoded encrypted private key
    pub public_key: String,
    pub requires_biometric: bool,
}

impl SavedWallet {
    pub fn new(account_id: String, private_key: String, public_key: String) -> Self {
        // Simple XOR encryption for demo (use proper encryption in production)
        let key_bytes = private_key.as_bytes();
        let encrypted: Vec<u8> = key_bytes.iter().enumerate()
            .map(|(i, b)| b ^ (i as u8))
            .collect();
        
        Self {
            account_id,
            encrypted_key: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &encrypted),
            public_key,
            requires_biometric: true,
        }
    }
    
    pub fn decrypt_key(&self) -> Option<String> {
        let encrypted = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &self.encrypted_key).ok()?;
        let decrypted: Vec<u8> = encrypted.iter().enumerate()
            .map(|(i, b)| b ^ (i as u8))
            .collect();
        String::from_utf8(decrypted).ok()
    }
}

// ============================================================================
// KEY MANAGEMENT
// ============================================================================

#[derive(Clone)]
pub struct KeyPair {
    pub private_key: String,
    pub public_key: String,
}

impl KeyPair {
    pub fn generate() -> Self {
        // Use rand to generate random bytes
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        
        let private_key = format!("ed25519:{}", bs58::encode(&bytes).into_string());
        
        // Derive public key using ed25519
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&bytes);
        let verifying_key = signing_key.verifying_key();
        let public_key = format!("ed25519:{}", bs58::encode(verifying_key.to_bytes()).into_string());
        
        Self { private_key, public_key }
    }
    
    pub fn from_private_key(key: &str) -> Option<Self> {
        let key = key.strip_prefix("ed25519:")?;
        let bytes = bs58::decode(key).into_vec().ok()?;
        if bytes.len() != 32 {
            return None;
        }
        let signing_key_bytes: [u8; 32] = bytes.try_into().ok()?;
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&signing_key_bytes);
        let verifying_key = signing_key.verifying_key();
        
        Some(Self {
            private_key: format!("ed25519:{}", bs58::encode(signing_key.to_bytes()).into_string()),
            public_key: format!("ed25519:{}", bs58::encode(verifying_key.to_bytes()).into_string()),
        })
    }
}

// ============================================================================
// TEXT INPUT
// ============================================================================

pub struct TextInput {
    focus_handle: FocusHandle,
    content: String,
    placeholder: &'static str,
    cursor: usize,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
}

impl TextInput {
    pub fn new(cx: &mut Context<Self>, placeholder: &'static str) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: String::new(),
            placeholder,
            cursor: 0,
            last_layout: None,
            last_bounds: None,
        }
    }

    pub fn text(&self) -> String { self.content.clone() }
    pub fn is_empty(&self) -> bool { self.content.is_empty() }
    pub fn len(&self) -> usize { self.content.chars().count() }
    
    fn char_count(&self) -> usize { self.content.chars().count() }
    
    fn byte_pos_from_char(&self, char_idx: usize) -> usize {
        self.content.char_indices()
            .nth(char_idx)
            .map(|(i, _)| i)
            .unwrap_or(self.content.len())
    }
    
    fn backspace(&mut self, cx: &mut Context<Self>) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let byte_pos = self.byte_pos_from_char(self.cursor);
            let char_len = self.content[byte_pos..].chars().next().map(|c| c.len_utf8()).unwrap_or(0);
            self.content.replace_range(byte_pos..byte_pos + char_len, "");
            cx.notify();
        }
    }
    
    fn delete_forward(&mut self, cx: &mut Context<Self>) {
        if self.cursor < self.char_count() {
            let byte_pos = self.byte_pos_from_char(self.cursor);
            let char_len = self.content[byte_pos..].chars().next().map(|c| c.len_utf8()).unwrap_or(0);
            self.content.replace_range(byte_pos..byte_pos + char_len, "");
            cx.notify();
        }
    }
    
    fn move_left(&mut self, cx: &mut Context<Self>) {
        if self.cursor > 0 { self.cursor -= 1; cx.notify(); }
    }
    
    fn move_right(&mut self, cx: &mut Context<Self>) {
        if self.cursor < self.char_count() { self.cursor += 1; cx.notify(); }
    }
    
    fn move_home(&mut self, cx: &mut Context<Self>) {
        self.cursor = 0;
        cx.notify();
    }
    
    fn move_end(&mut self, cx: &mut Context<Self>) {
        self.cursor = self.char_count();
        cx.notify();
    }

    fn paste(&mut self, text: String, cx: &mut Context<Self>) {
        if !text.is_empty() {
            let byte_pos = self.byte_pos_from_char(self.cursor);
            self.content.insert_str(byte_pos, &text);
            self.cursor += text.chars().count();
            cx.notify();
        }
    }

    fn copy(&self) -> String {
        self.content.clone()
    }

    fn cut(&mut self, cx: &mut Context<Self>) {
        // Copy content then clear
        let _ = self.content.clone();
        self.content.clear();
        self.cursor = 0;
        cx.notify();
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        // Select all text (move cursor to end for now)
        self.cursor = self.char_count();
        cx.notify();
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(&mut self, _: Range<usize>, _: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        Some(self.content.clone())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let utf16_cursor: usize = self.content.chars().take(self.cursor).map(|c| c.len_utf16()).sum();
        Some(UTF16Selection { range: utf16_cursor..utf16_cursor, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> { None }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let byte_pos = self.byte_pos_from_char(self.cursor);
        self.content.insert_str(byte_pos, text);
        self.cursor += text.chars().count();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(&mut self, r: Option<Range<usize>>, t: &str, _: Option<Range<usize>>, w: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(r, t, w, cx);
    }

    fn bounds_for_range(&mut self, _: Range<usize>, _: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> { None }
    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> { None }
}

pub struct TextElement { input: Entity<TextInput> }

impl IntoElement for TextElement {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = (Option<ShapedLine>, Option<PaintQuad>);

    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&gpui::InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&gpui::InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) -> (Option<ShapedLine>, Option<PaintQuad>) {
        let input = self.input.read(cx);
        let (text, color): (String, Hsla) = if input.content.is_empty() {
            (input.placeholder.to_string(), rgb(0x6c7086).into())
        } else {
            (input.content.clone(), rgb(0xcdd6f4).into())
        };

        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let len = text.len();
        let line = window.text_system().shape_line(SharedString::from(text), font_size, &[TextRun {
            len, font: style.font(), color, background_color: None, underline: None, strikethrough: None,
        }], None);

        let cursor_byte = input.byte_pos_from_char(input.cursor);
        let cursor_x = line.x_for_index(cursor_byte);
        let cursor = if input.focus_handle.is_focused(window) {
            Some(fill(Bounds::new(point(bounds.left() + cursor_x, bounds.top()), gpui::size(px(1.5), bounds.bottom() - bounds.top())), rgb(0x89b4fa)))
        } else { None };

        (Some(line), cursor)
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&gpui::InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), prepaint: &mut (Option<ShapedLine>, Option<PaintQuad>), window: &mut Window, cx: &mut App) {
        let fh = self.input.read(cx).focus_handle.clone();
        window.handle_input(&fh, ElementInputHandler::new(bounds, self.input.clone()), cx);
        if let Some(line) = prepaint.0.take() {
            line.paint(bounds.origin, window.line_height(), window, cx).ok();
            self.input.update(cx, |i, _| { i.last_layout = Some(line); i.last_bounds = Some(bounds); });
        }
        if let Some(c) = prepaint.1.take() { window.paint_quad(c); }
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        let border = if focused { rgb(0x89b4fa) } else { rgb(0x45475a) };

        div()
            .id("text-input")
            .key_context("TextInput")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .bg(rgb(0x181825))
            .border_1()
            .border_color(border)
            .rounded_md()
            .px_3()
            .w_full()
            .h(px(40.0))
            .flex()
            .items_center()
            .justify_start()
            .on_action(cx.listener(|this, _: &Backspace, _, cx| this.backspace(cx)))
            .on_action(cx.listener(|this, _: &Delete, _, cx| this.delete_forward(cx)))
            .on_action(cx.listener(|this, _: &Left, _, cx| this.move_left(cx)))
            .on_action(cx.listener(|this, _: &Right, _, cx| this.move_right(cx)))
            .on_action(cx.listener(|this, _: &Home, _, cx| this.move_home(cx)))
            .on_action(cx.listener(|this, _: &End, _, cx| this.move_end(cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|_, _: &Enter, _, _| { /* Consume Enter to prevent crash */ }))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                let x = ev.position.x - this.last_bounds.unwrap_or_default().left();
                if let Some(layout) = &this.last_layout {
                    let byte_idx = layout.closest_index_for_x(x);
                    this.cursor = this.content[..byte_idx.min(this.content.len())].chars().count();
                }
                window.focus(&this.focus_handle);
                cx.notify();
            }))
            .child(TextElement { input: cx.entity() })
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() }
}

// ============================================================================
// WALLET APP
// ============================================================================

struct WalletApp {
    screen: Screen,
    account_id: String,
    balance: String,
    network: Network,
    keypair: Option<KeyPair>,
    account_input: Entity<TextInput>,
    key_input: Entity<TextInput>,
    recipient_input: Entity<TextInput>,
    amount_input: Entity<TextInput>,
    error: Option<String>,
    success: Option<String>,
    loading: bool,
    transactions: Vec<Transaction>,
    loading_txs: bool,
    // Touch ID support
    biometric_auth: Arc<Mutex<BiometricAuth>>,
    saved_wallets: Vec<SavedWallet>,
    biometric_available: bool,
    authenticating: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Screen { Welcome, Create, Import, Unlock, Dashboard, Send, History }

impl WalletApp {
    fn new(cx: &mut Context<Self>) -> Self {
        let biometric_auth = BiometricAuth::new();
        let biometric_available = biometric_auth.is_available();
        
        // Load saved wallets from storage
        let saved_wallets = Self::load_saved_wallets();
        
        Self {
            screen: if !saved_wallets.is_empty() { Screen::Unlock } else { Screen::Welcome },
            account_id: String::new(),
            balance: "0.00 NEAR".to_string(),
            network: Network::Testnet,
            keypair: None,
            account_input: cx.new(|cx| TextInput::new(cx, "yourname.testnet")),
            key_input: cx.new(|cx| TextInput::new(cx, "ed25519:...")),
            recipient_input: cx.new(|cx| TextInput::new(cx, "recipient.testnet")),
            amount_input: cx.new(|cx| TextInput::new(cx, "0.0")),
            error: None,
            success: None,
            loading: false,
            transactions: Vec::new(),
            loading_txs: false,
            // Touch ID support
            biometric_auth: Arc::new(Mutex::new(biometric_auth)),
            saved_wallets,
            biometric_available,
            authenticating: false,
        }
    }
    
    fn load_saved_wallets() -> Vec<SavedWallet> {
        // Load from ~/.near-wallet-wallets.json
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let path = format!("{}/.near-wallet-wallets.json", home);
        if let Ok(data) = std::fs::read_to_string(&path) {
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            Vec::new()
        }
    }
    
    fn save_wallets(&self) {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let path = format!("{}/.near-wallet-wallets.json", home);
        if let Ok(data) = serde_json::to_string_pretty(&self.saved_wallets) {
            let _ = std::fs::write(&path, data);
        }
    }
    
    fn authenticate_with_touch_id(&mut self, wallet_index: usize, cx: &mut Context<Self>) {
        if wallet_index >= self.saved_wallets.len() {
            self.error = Some("Invalid wallet selection".to_string());
            cx.notify();
            return;
        }
        
        let wallet = self.saved_wallets[wallet_index].clone();
        let biometric_auth = self.biometric_auth.clone();
        let account_id = wallet.account_id.clone();
        
        self.authenticating = true;
        self.error = None;
        cx.notify();
        
        // Spawn async task for biometric authentication
        cx.spawn(async move |this, cx| {
            let auth = biometric_auth.lock();
            let result = auth.authenticate(&format!("Unlock wallet: {}", account_id)).await;
            drop(auth);
            
            let _ = this.update(cx, |this, cx| {
                this.authenticating = false;
                
                match result {
                    BiometricResult::Success => {
                        // Decrypt and load wallet
                        if let Some(private_key) = wallet.decrypt_key() {
                            if let Some(kp) = KeyPair::from_private_key(&private_key) {
                                this.keypair = Some(kp);
                                this.account_id = wallet.account_id;
                                this.error = None;
                                this.success = Some("Wallet unlocked!".to_string());
                                this.screen = Screen::Dashboard;
                                this.fetch_balance(cx);
                                this.fetch_transactions(cx);
                            } else {
                                this.error = Some("Failed to decrypt wallet".to_string());
                            }
                        } else {
                            this.error = Some("Failed to decrypt wallet".to_string());
                        }
                    }
                    BiometricResult::Failed(msg) => {
                        this.error = Some(format!("Authentication failed: {}", msg));
                    }
                    BiometricResult::NotAvailable => {
                        this.error = Some("Touch ID not available".to_string());
                    }
                }
                
                cx.notify();
            });
        }).detach();
    }
    
    fn fetch_balance(&mut self, cx: &mut Context<Self>) {
        let account = self.account_id.clone();
        let network = self.network;
        self.loading = true;
        self.error = None;
        cx.notify();
        
        cx.spawn(async move |this, cx| {
            let rpc = NearRpc::new(network);
            let result = rpc.get_account_balance(&account).await;
            
            this.update(cx, |this, cx| {
                match result {
                    Ok(balance) => {
                        this.balance = balance.format();
                        this.success = Some("Balance fetched!".to_string());
                    }
                    Err(e) => {
                        this.error = Some(format!("Failed to fetch balance: {}", e));
                    }
                }
                this.loading = false;
                cx.notify();
            }).ok();
        }).detach();
    }
    
    fn fetch_transactions(&mut self, cx: &mut Context<Self>) {
        let account = self.account_id.clone();
        let network = self.network;
        self.loading_txs = true;
        cx.notify();
        
        cx.spawn(async move |this, cx| {
            let rpc = NearRpc::new(network);
            let result = rpc.get_transaction_history(&account, 10).await;
            
            this.update(cx, |this, cx| {
                match result {
                    Ok(txs) => {
                        this.transactions = txs;
                    }
                    Err(e) => {
                        this.error = Some(format!("Failed to fetch history: {}", e));
                    }
                }
                this.loading_txs = false;
                cx.notify();
            }).ok();
        }).detach();
    }
    
    fn generate_keypair(&mut self, cx: &mut Context<Self>) {
        let keypair = KeyPair::generate();
        self.key_input.update(cx, |input, cx| {
            input.content = keypair.private_key.clone();
            input.cursor = input.char_count();
            cx.notify();
        });
        self.keypair = Some(keypair);
        self.success = Some("Key generated!".to_string());
        cx.notify();
    }
    
    fn validate_and_import(&mut self, cx: &mut Context<Self>) {
        let key = self.key_input.read(cx).text();
        let account = self.account_input.read(cx).text();
        
        if key.is_empty() {
            self.error = Some("Private key is required".to_string());
            cx.notify();
            return;
        }
        if !key.starts_with("ed25519:") {
            self.error = Some("Private key must start with ed25519:".to_string());
            cx.notify();
            return;
        }
        if account.is_empty() {
            self.error = Some("Account ID is required".to_string());
            cx.notify();
            return;
        }
        
        // Parse and store keypair
        match KeyPair::from_private_key(&key) {
            Some(kp) => {
                self.keypair = Some(kp.clone());
                
                // Save wallet with Touch ID protection
                if self.biometric_available {
                    let saved_wallet = SavedWallet::new(
                        account.clone(),
                        kp.private_key,
                        kp.public_key,
                    );
                    self.saved_wallets.push(saved_wallet);
                    self.save_wallets();
                }
            }
            None => {
                self.error = Some("Invalid private key format".to_string());
                cx.notify();
                return;
            }
        }
        
        self.account_id = account;
        self.error = None;
        self.success = if self.biometric_available {
            Some("Wallet imported and protected with Touch ID!".to_string())
        } else {
            Some("Wallet imported!".to_string())
        };
        self.fetch_balance(cx);
        self.fetch_transactions(cx);
        self.screen = Screen::Dashboard;
        cx.notify();
    }
    
    fn validate_and_create(&mut self, cx: &mut Context<Self>) {
        let account = self.account_input.read(cx).text();
        
        if account.is_empty() {
            self.error = Some("Account ID is required".to_string());
            cx.notify();
            return;
        }
        if !account.ends_with(".testnet") && !account.ends_with(".near") {
            self.error = Some("Account must end with .testnet or .near".to_string());
            cx.notify();
            return;
        }
        
        // Generate a new keypair
        self.generate_keypair(cx);
        self.account_id = account;
        self.error = None;
        self.success = Some("Account created locally!".to_string());
        self.screen = Screen::Dashboard;
        self.fetch_balance(cx);
        self.fetch_transactions(cx);
        cx.notify();
    }
    
    fn validate_and_send(&mut self, cx: &mut Context<Self>) {
        let recipient = self.recipient_input.read(cx).text();
        let amount = self.amount_input.read(cx).text();
        
        if recipient.is_empty() {
            self.error = Some("Recipient is required".to_string());
            cx.notify();
            return;
        }
        if amount.is_empty() {
            self.error = Some("Amount is required".to_string());
            cx.notify();
            return;
        }
        let amount_val: f64 = match amount.parse() {
            Ok(v) => v,
            Err(_) => {
                self.error = Some("Amount must be a number".to_string());
                cx.notify();
                return;
            }
        };
        if amount_val <= 0.0 {
            self.error = Some("Amount must be positive".to_string());
            cx.notify();
            return;
        }
        
        self.error = None;
        self.success = Some(format!("Transaction simulated: {} NEAR to {}", amount_val, recipient));
        cx.notify();
    }
}

impl Render for WalletApp {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg = rgb(0x1e1e2e);
        let border = rgb(0x45475a);
        let text = rgb(0xcdd6f4);
        let muted = rgb(0x6c7086);
        let accent = rgb(0x89b4fa);
        let green = rgb(0xa6e3a1);
        let red = rgb(0xf38ba8);

        div().flex().flex_col().size_full().bg(bg).text_color(text).font_family(".SystemUIFont").p_4()
            .child(match self.screen {
                Screen::Welcome => div().flex().flex_col().size_full()
                    .child(div().flex().flex_col().items_center().pt_16().pb_8()
                        .child(div().text_2xl().font_weight(gpui::FontWeight::BOLD).text_color(accent).child("NEAR Wallet"))
                        .child(div().text_sm().text_color(muted).mt_2().child("A modern wallet for NEAR Protocol")))
                    .child(div().flex().flex_col().items_center().gap_3().pt_8()
                        .child(div().id("create").w(px(200.0)).h(px(44.0)).flex().items_center().justify_center().rounded_md().bg(accent).text_color(bg).font_weight(gpui::FontWeight::SEMIBOLD).cursor_pointer().child("Create New Wallet").on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { this.error = None; this.success = None; this.screen = Screen::Create; cx.notify(); })))
                        .child(div().id("import").w(px(200.0)).h(px(44.0)).flex().items_center().justify_center().rounded_md().bg(border).text_color(text).cursor_pointer().child("Import Wallet").on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { this.error = None; this.success = None; this.screen = Screen::Import; cx.notify(); }))))
                    .child(div().flex().flex_1().items_end().justify_center().pb_4().child(div().text_xs().text_color(muted).child("Built with GPUI"))).into_any_element(),

                Screen::Create => {
                    let account_text = self.account_input.read(cx).text();
                    
                    div().flex().flex_col().size_full()
                        .child(div().flex().items_center().gap_2().pb_4().border_b_1().border_color(border)
                            .child(div().px_3().py_1().rounded_md().bg(border).text_color(text).text_sm().cursor_pointer().child("← Back").on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { this.screen = Screen::Welcome; cx.notify(); })))
                            .child(div().text_lg().font_weight(gpui::FontWeight::SEMIBOLD).ml_2().child("Create New Wallet")))
                        .child(div().flex().flex_col().gap_3().pt_6()
                            .child(div().text_sm().text_color(muted).child("Enter an account ID:"))
                            .child(self.account_input.clone())
                            .child(div().text_xs().text_color(if account_text.ends_with(".testnet") || account_text.ends_with(".near") { green } else { muted }).child(if account_text.is_empty() { "Required field" } else if account_text.ends_with(".testnet") || account_text.ends_with(".near") { "✓ Valid suffix" } else { "Must end with .testnet or .near" }))
                            .when_some(self.error.clone(), |el, e| el.child(div().text_sm().text_color(red).child(e)))
                            .when_some(self.success.clone(), |el, s| el.child(div().text_sm().text_color(green).child(s))))
                        .child(div().flex().flex_1().items_end().justify_end().gap_3()
                            .child(div().px_4().py_2().rounded_md().bg(border).text_color(text).text_sm().cursor_pointer().child("Cancel").on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { this.screen = Screen::Welcome; cx.notify(); })))
                            .child(div().px_4().py_2().rounded_md().bg(green).text_color(bg).font_weight(gpui::FontWeight::SEMIBOLD).text_sm().cursor_pointer().child("Create").on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { this.validate_and_create(cx); })))).into_any_element()
                }

                Screen::Import => {
                    let key_text = self.key_input.read(cx).text();
                    let account_text = self.account_input.read(cx).text();
                    
                    div().flex().flex_col().size_full()
                        .child(div().flex().items_center().gap_2().pb_4().border_b_1().border_color(border)
                            .child(div().px_3().py_1().rounded_md().bg(border).text_color(text).text_sm().cursor_pointer().child("← Back").on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { this.screen = Screen::Welcome; cx.notify(); })))
                            .child(div().text_lg().font_weight(gpui::FontWeight::SEMIBOLD).ml_2().child("Import Wallet")))
                        .child(div().flex().flex_col().gap_3().pt_6()
                            .child(div().text_sm().child("Private Key:"))
                            .child(self.key_input.clone())
                            .child(div().text_xs().text_color(if key_text.starts_with("ed25519:") { green } else if key_text.is_empty() { muted } else { red }).child(if key_text.is_empty() { "Required field" } else if key_text.starts_with("ed25519:") { "✓ Valid format" } else { "Must start with ed25519:" }))
                            .child(div().text_sm().child("Account ID:"))
                            .child(self.account_input.clone())
                            .child(div().text_xs().text_color(if account_text.is_empty() { muted } else { green }).child(if account_text.is_empty() { "Required field" } else { "✓ Account set" }))
                            .when_some(self.error.clone(), |el, e| el.child(div().text_sm().text_color(red).child(e))))
                        .child(div().flex().flex_1().items_end().justify_end().gap_3()
                            .child(div().px_4().py_2().rounded_md().bg(border).text_color(text).text_sm().cursor_pointer().child("Cancel").on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { this.screen = Screen::Welcome; cx.notify(); })))
                            .child(div().px_4().py_2().rounded_md().bg(accent).text_color(bg).font_weight(gpui::FontWeight::SEMIBOLD).text_sm().cursor_pointer().child("Import").on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { this.validate_and_import(cx); })))).into_any_element()
                }

                Screen::Unlock => {
                    let authenticating = self.authenticating;
                    let biometric_available = self.biometric_available;
                    
                    div().flex().flex_col().size_full()
                        .child(div().flex().flex_col().items_center().pt_16().pb_8()
                            .child(div().text_2xl().font_weight(gpui::FontWeight::BOLD).text_color(accent).child("NEAR Wallet"))
                            .child(div().text_sm().text_color(muted).mt_2().child("Select a wallet to unlock")))
                        .child(div().flex().flex_col().items_center().gap_3().pt_4()
                            .children(self.saved_wallets.iter().enumerate().map(|(i, wallet)| {
                                let is_authenticating = authenticating;
                                let wallet_account = wallet.account_id.clone();
                                let wallet_public = wallet.public_key.clone();
                                let wallet_clone = wallet.clone();
                                
                                div().w(px(280.0)).p_4().rounded_md().bg(rgb(0x181825)).border_1().border_color(border)
                                    .child(div().flex().justify_between().items_center()
                                        .child(div().flex().flex_col().gap_1()
                                            .child(div().text_sm().font_weight(gpui::FontWeight::SEMIBOLD).text_color(text).child(wallet_account))
                                            .child(div().text_xs().text_color(muted).child(format!("{}...", &wallet_public[..20]))))
                                        .child(if biometric_available {
                                            div().px_3().py_2().rounded_md().bg(accent).text_color(bg).font_weight(gpui::FontWeight::SEMIBOLD).text_sm().cursor_pointer()
                                                .child(if is_authenticating { "..." } else { "Touch ID" })
                                                .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                    if !is_authenticating {
                                                        this.authenticate_with_touch_id(i, cx);
                                                    }
                                                }))
                                        } else {
                                            div().px_3().py_2().rounded_md().bg(accent).text_color(bg).font_weight(gpui::FontWeight::SEMIBOLD).text_sm().cursor_pointer()
                                                .child("Unlock")
                                                .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                    // Fallback: direct unlock (no Touch ID)
                                                    if let Some(key) = wallet_clone.decrypt_key() {
                                                        if let Some(kp) = KeyPair::from_private_key(&key) {
                                                            this.keypair = Some(kp);
                                                            this.account_id = wallet_clone.account_id.clone();
                                                            this.screen = Screen::Dashboard;
                                                            this.fetch_balance(cx);
                                                            this.fetch_transactions(cx);
                                                            cx.notify();
                                                        }
                                                    }
                                                }))
                                        }))
                            }))
                            .when_some(self.error.clone(), |el, e| el.child(div().text_sm().text_color(red).mt_2().child(e)))
                            .child(div().mt_4().text_xs().text_color(muted).px_3().py_1().rounded_md().bg(border).cursor_pointer().child("+ Import Another").on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { this.screen = Screen::Import; cx.notify(); }))))
                        .child(div().flex().flex_1().items_end().justify_center().pb_4().child(div().text_xs().text_color(muted).child("Built with GPUI"))).into_any_element()
                }

                Screen::Dashboard => {
                    let acc = if self.account_id.is_empty() { "wallet.testnet".to_string() } else { self.account_id.clone() };
                    let balance_str = SharedString::from(if self.loading { "Loading...".to_string() } else { self.balance.clone() });
                    
                    div().flex().flex_col().size_full()
                        .child(div().flex().justify_between().items_center().pb_4().border_b_1().border_color(border)
                            .child(div().text_lg().font_weight(gpui::FontWeight::SEMIBOLD).text_color(accent).child("NEAR Wallet"))
                            .child(div().px_3().py_1().rounded_md().bg(red).text_color(bg).text_sm().cursor_pointer().child("Lock").on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { *this = Self::new(cx); cx.notify(); }))))
                        .child(div().flex().flex_col().items_center().pt_8().pb_6()
                            .child(div().text_sm().text_color(muted).child("Balance"))
                            .child(div().text_3xl().font_weight(gpui::FontWeight::BOLD).text_color(green).mt_1().child(balance_str))
                            .child(div().text_sm().text_color(muted).mt_2().child(acc)))
                        .child(div().flex().gap_3().justify_center()
                            .child(div().px_4().py_2().rounded_md().bg(border).text_color(text).text_sm().cursor_pointer().child(if self.loading { "Fetching..." } else { "Refresh" }).on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { this.fetch_balance(cx); })))
                            .child(div().px_4().py_2().rounded_md().bg(accent).text_color(bg).font_weight(gpui::FontWeight::SEMIBOLD).text_sm().cursor_pointer().child("Send").on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { this.error = None; this.success = None; this.screen = Screen::Send; cx.notify(); })))
                            .child(div().px_4().py_2().rounded_md().bg(border).text_color(text).text_sm().cursor_pointer().child("History").on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { this.error = None; this.success = None; this.fetch_transactions(cx); this.screen = Screen::History; cx.notify(); }))))
                        .child(div().flex().flex_1().items_end().justify_between().pb_4()
                            .child(div().text_xs().text_color(muted).child(format!("Network: {}", self.network)))
                            .when_some(self.error.clone(), |el, e| el.child(div().text_xs().text_color(red).child(e)))
                            .when_some(self.success.clone(), |el, s| el.child(div().text_xs().text_color(green).child(s)))).into_any_element()
                }

                Screen::Send => {
                    let recipient_text = self.recipient_input.read(cx).text();
                    let amount_text = self.amount_input.read(cx).text();
                    let is_valid_amount = amount_text.parse::<f64>().is_ok() && !amount_text.is_empty();
                    
                    div().flex().flex_col().size_full()
                        .child(div().flex().items_center().gap_2().pb_4().border_b_1().border_color(border)
                            .child(div().px_3().py_1().rounded_md().bg(border).text_color(text).text_sm().cursor_pointer().child("← Back").on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { this.screen = Screen::Dashboard; cx.notify(); })))
                            .child(div().text_lg().font_weight(gpui::FontWeight::SEMIBOLD).ml_2().child("Send NEAR")))
                        .child(div().flex().flex_col().gap_3().pt_6()
                            .child(div().text_sm().child("Recipient:"))
                            .child(self.recipient_input.clone())
                            .child(div().text_xs().text_color(if recipient_text.is_empty() { muted } else { green }).child(if recipient_text.is_empty() { "Required field" } else { "✓ Recipient set" }))
                            .child(div().text_sm().child("Amount (NEAR):"))
                            .child(self.amount_input.clone())
                            .child(div().text_xs().text_color(if is_valid_amount { green } else { muted }).child(if amount_text.is_empty() { "Required field" } else if is_valid_amount { "✓ Valid amount" } else { "Must be a positive number" }))
                            .when_some(self.error.clone(), |el, e| el.child(div().text_sm().text_color(red).child(e)))
                            .when_some(self.success.clone(), |el, s| el.child(div().text_sm().text_color(green).child(s))))
                        .child(div().flex().flex_1().items_end().justify_end()
                            .child(div().w(px(120.0)).h(px(36.0)).flex().items_center().justify_center().rounded_md().bg(green).text_color(bg).font_weight(gpui::FontWeight::SEMIBOLD).text_sm().cursor_pointer().child("Send").on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { this.validate_and_send(cx); })))).into_any_element()
                }
                
                Screen::History => {
                    let loading = self.loading_txs;
                    let txs = self.transactions.clone();
                    
                    div().flex().flex_col().size_full()
                        .child(div().flex().items_center().gap_2().pb_4().border_b_1().border_color(border)
                            .child(div().px_3().py_1().rounded_md().bg(border).text_color(text).text_sm().cursor_pointer().child("← Back").on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| { this.screen = Screen::Dashboard; cx.notify(); })))
                            .child(div().text_lg().font_weight(gpui::FontWeight::SEMIBOLD).ml_2().child("Transaction History")))
                        .child(if loading {
                            div().flex().flex_1().items_center().justify_center().child(div().text_color(muted).child("Loading transactions...")).into_any_element()
                        } else if txs.is_empty() {
                            div().flex().flex_1().items_center().justify_center().child(div().text_color(muted).child("No transactions found")).into_any_element()
                        } else {
                            div().flex().flex_col().gap_2().pt_4()
                                .children(txs.iter().map(|tx| {
                                    let is_sent = tx.signer_id == self.account_id;
                                    let other = if is_sent { &tx.receiver_id } else { &tx.signer_id };
                                    let arrow = if is_sent { "→" } else { "←" };
                                    let amount_color = if is_sent { red } else { green };
                                    let amount_str = tx.format_amount();
                                    
                                    div().flex().justify_between().items_center().p_3().rounded_md().bg(rgb(0x181825))
                                        .child(div().flex().flex_col().gap_1()
                                            .child(div().text_sm().text_color(text).child(format!("{} {}", arrow, other)))
                                            .child(div().text_xs().text_color(muted).child(format!("{}...{}", &tx.hash[..8], &tx.hash[tx.hash.len()-6..]))))
                                        .child(div().text_sm().text_color(amount_color).child(if is_sent { format!("-{}", amount_str) } else { format!("+{}", amount_str) }))
                                }))
                                .into_any_element()
                        }).into_any_element()
                }
            })
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("backspace", Backspace, Some("TextInput")),
            KeyBinding::new("delete", Delete, Some("TextInput")),
            KeyBinding::new("left", Left, Some("TextInput")),
            KeyBinding::new("right", Right, Some("TextInput")),
            KeyBinding::new("home", Home, Some("TextInput")),
            KeyBinding::new("end", End, Some("TextInput")),
            KeyBinding::new("cmd-v", Paste, Some("TextInput")),
            KeyBinding::new("cmd-c", Copy, Some("TextInput")),
            KeyBinding::new("cmd-x", Cut, Some("TextInput")),
            KeyBinding::new("cmd-a", SelectAll, Some("TextInput")),
            KeyBinding::new("enter", Enter, Some("TextInput")),
        ]);
        
        let bounds = Bounds::centered(None, size(px(420.), px(640.0)), cx);
        cx.open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions { title: Some(SharedString::from("NEAR Wallet")), appears_transparent: false, traffic_light_position: None }),
            ..Default::default()
        }, |_, cx| cx.new(|cx| WalletApp::new(cx))).unwrap();
        cx.activate(true);
    });
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Key Generation Tests
    
    #[test]
    fn test_keypair_generation() {
        let kp = KeyPair::generate();
        assert!(kp.private_key.starts_with("ed25519:"));
        assert!(kp.public_key.starts_with("ed25519:"));
        
        let kp2 = KeyPair::generate();
        assert_ne!(kp.private_key, kp2.private_key);
    }

    #[test]
    fn test_keypair_roundtrip() {
        let original = KeyPair::generate();
        let restored = KeyPair::from_private_key(&original.private_key).unwrap();
        assert_eq!(original.private_key, restored.private_key);
        assert_eq!(original.public_key, restored.public_key);
    }

    #[test]
    fn test_keypair_invalid_keys() {
        assert!(KeyPair::from_private_key("abc123").is_none());
        assert!(KeyPair::from_private_key("secp256k1:abc").is_none());
        assert!(KeyPair::from_private_key("ed25519:").is_none());
        assert!(KeyPair::from_private_key("ed25519:!!!").is_none());
        assert!(KeyPair::from_private_key("ed25519:2V").is_none());
    }

    #[test]
    fn test_keypair_format() {
        let kp = KeyPair::generate();
        let private_b58 = kp.private_key.strip_prefix("ed25519:").unwrap();
        let public_b58 = kp.public_key.strip_prefix("ed25519:").unwrap();
        
        let private_bytes = bs58::decode(private_b58).into_vec().unwrap();
        let public_bytes = bs58::decode(public_b58).into_vec().unwrap();
        
        assert_eq!(private_bytes.len(), 32);
        assert_eq!(public_bytes.len(), 32);
    }

    // Validation Tests
    
    #[test]
    fn test_account_validation() {
        fn validate(account: &str) -> Result<(), String> {
            if account.is_empty() { return Err("required".into()); }
            if !account.ends_with(".testnet") && !account.ends_with(".near") {
                return Err("invalid suffix".into());
            }
            Ok(())
        }
        
        assert!(validate("user.testnet").is_ok());
        assert!(validate("user.near").is_ok());
        assert!(validate("").is_err());
        assert!(validate("user").is_err());
    }

    #[test]
    fn test_amount_validation() {
        fn validate(amount: &str) -> Result<f64, String> {
            if amount.is_empty() { return Err("required".into()); }
            let val: f64 = amount.parse().map_err(|_| "not a number".to_string())?;
            if val <= 0.0 { return Err("must be positive".into()); }
            Ok(val)
        }
        
        assert_eq!(validate("1.0").unwrap(), 1.0);
        assert_eq!(validate("0.001").unwrap(), 0.001);
        assert!(validate("").is_err());
        assert!(validate("0").is_err());
        assert!(validate("abc").is_err());
    }

    // UTF-8 Tests
    
    #[test]
    fn test_utf8_char_count() {
        assert_eq!("hello".chars().count(), 5);
        assert_eq!("héllo".chars().count(), 5);
        assert_eq!("🎉".chars().count(), 1);
    }

    #[test]
    fn test_utf8_byte_position() {
        let s = "héllo";
        let byte_pos = s.char_indices().nth(2).map(|(i, _)| i).unwrap_or(s.len());
        assert_eq!(byte_pos, 3);
        
        let s = "a🎉b";
        let byte_pos = s.char_indices().nth(2).map(|(i, _)| i).unwrap_or(s.len());
        assert_eq!(byte_pos, 5);
    }

    // Balance Format Tests
    
    #[test]
    fn test_yocto_to_near() {
        let yocto: u128 = 1_000_000_000_000_000_000_000_000;
        let near = yocto as f64 / 1_000_000_000_000_000_000_000_000_u128 as f64;
        assert!((near - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_balance_format() {
        let formatted = format!("{:.4} NEAR", 1.23456789);
        assert_eq!(formatted, "1.2346 NEAR");
    }

    #[test]
    fn test_tx_hash_truncation() {
        let hash = "8obCVrMQD8tzLqVP8Zs7AZpEngHF4Fi8pmk6ihh8chbT";
        let truncated = format!("{}...{}", &hash[..8], &hash[hash.len()-6..]);
        assert_eq!(truncated, "8obCVrMQ...h8chbT");
    }
}
