//! UI localization (extensible dropdown language list; English + YouTube-authentic Turkish).

/// Languages available for the desktop client interface.
pub const APP_LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("tr", "Türkçe"),
];

pub fn is_tr() -> bool {
    crate::store::settings_snapshot().app_language.eq_ignore_ascii_case("tr")
}

pub fn tr(en: &'static str, tr: &'static str) -> &'static str {
    if is_tr() { tr } else { en }
}

/// Formats comment reply counts ("3 replies" -> "3 yanıt").
pub fn reply_count(count: &str) -> String {
    if is_tr() {
        format!("{count} yanıt")
    } else if count == "1" {
        format!("{count} reply")
    } else {
        format!("{count} replies")
    }
}

/// Translates a UI string into the active app language, returning the original if unchanged.
pub fn t(en: &str) -> String {
    if !is_tr() {
        return en.to_string();
    }
    if let Some(v) = lookup_tr(en) {
        return v.to_string();
    }
    // Dynamic patterns
    if let Some(rest) = en.strip_prefix("Skip ") {
        return format!("Atla: {}", t(rest));
    }
    if let Some(rest) = en.strip_prefix("Subscribed to ") {
        return format!("{rest} kanalına abone olundu");
    }
    if let Some(rest) = en.strip_prefix("Unsubscribed from ") {
        return format!("{rest} aboneliğinden çıkıldı");
    }
    if let Some(rest) = en.strip_prefix("Profile: ") {
        return format!("Profil: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Updated ") {
        return format!("Güncellendi: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Up next: ") {
        return format!("Sıradaki: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Detected yt-dlp ") {
        return format!("yt-dlp bulundu ({rest})");
    }
    if let Some(rest) = en.strip_prefix("Version ") {
        return format!("Sürüm {rest}");
    }
    if let Some(rest) = en.strip_prefix("Speed ") {
        return format!("Hız {rest}");
    }
    if let Some(rest) = en.strip_prefix("Get yt-dlp (").and_then(|s| s.strip_suffix(')')) {
        return format!("yt-dlp indir ({rest})");
    }
    if let Some(rest) = en.strip_prefix("Get FFmpeg (").and_then(|s| s.strip_suffix(')')) {
        return format!("FFmpeg indir ({rest})");
    }
    if let Some(rest) = en.strip_prefix("Downloading… ") {
        return format!("İndiriliyor… {rest}");
    }
    if let Some(rest) = en.strip_prefix("Saved to ") {
        return format!("Kaydedildi: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Exported to ") {
        return format!("Dışa aktarıldı: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Failed: ") {
        return format!("Başarısız: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Import failed: ") {
        return format!("İçe aktarma başarısız: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Export failed: ") {
        return format!("Dışa aktarma başarısız: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Could not check: ") {
        return format!("Denetlenemedi: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Could not load comments: ") {
        return format!("Yorumlar yüklenemedi: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Could not load video details: ") {
        return format!("Video ayrıntıları yüklenemedi: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Couldn't load stats: ") {
        return format!("İstatistikler yüklenemedi: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Copy link ") {
        return format!("Bağlantıyı kopyala: {rest}");
    }
    if let Some(rest) = en.strip_suffix(" is available") {
        return format!("{rest} sürümü hazır");
    }
    if let Some(rest) = en.strip_suffix(" videos") {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '.') {
            return format!("{rest} video");
        }
    }
    if let Some(rest) = en.strip_prefix("Subscriptions (").and_then(|s| s.strip_suffix(')')) {
        return format!("Abonelikler ({rest})");
    }
    if let Some(rest) = en.strip_prefix("Results for “").and_then(|s| s.strip_suffix('”')) {
        return format!("“{rest}” için sonuçlar");
    }
    if let Some(rest) = en.strip_prefix("Installed (").and_then(|s| s.strip_suffix("). Downloads in every quality, with SponsorBlock chapters")) {
        return format!("Yüklü ({rest}). Tüm kalitelerde ve SponsorBlock bölümleriyle indirme yapılabilir");
    }
    if let Some(rest) = en.strip_prefix("Leave empty to use ").and_then(|s| s.strip_suffix(", which is installed. Or enter the path to another player")) {
        return format!("Yüklü olan {rest} oynatıcısını kullanmak için boş bırakın veya başka bir oynatıcının yolunu girin");
    }
    if let Some(rest) = en.strip_prefix("Connected as ").and_then(|s| s.strip_suffix(". Videos you watch here show in your YouTube history and resume where you stopped on your other devices")) {
        return format!("{rest} olarak bağlandı. Burada izlediğiniz videolar YouTube geçmişinizde görünür ve diğer cihazlarınızda kaldığınız yerden devam eder");
    }
    if let Some(rest) = en.strip_prefix("Source: ") {
        return format!("Kaynak: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Removed from ") {
        return format!("{rest} listesinden kaldırıldı");
    }
    if let Some(rest) = en.strip_prefix("Added ").and_then(|s| s.strip_suffix(" subscriptions")) {
        return format!("{rest} abonelik eklendi");
    }
    if let Some(rest) = en.strip_suffix(" channels could not be refreshed") {
        return format!("{rest} kanal yenilenemedi");
    }
    if let Some(rest) = en.strip_prefix("You have ") {
        return format!("Mevcut sürüm: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Could not load your YouTube playlists: ") {
        return format!("YouTube oynatma listeleriniz yüklenemedi: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Couldn't remove it from YouTube: ") {
        return format!("YouTube'dan kaldırılamadı: {rest}");
    }
    if let Some(rest) = en.strip_prefix("Connected as ").and_then(|s| s.strip_suffix(". What you watch now shows in your YouTube history.")) {
        return format!("{rest} olarak bağlandı. İzlediğiniz videolar artık YouTube geçmişinizde görünecek.");
    }
    if let Some(rest) = en.strip_suffix("x speed") {
        return format!("{rest}x hız");
    }
    if let Some(rest) = en.strip_prefix("Refreshing ").and_then(|s| s.strip_suffix('…')) {
        return format!("Yenileniyor {rest}…");
    }
    en.to_string()
}

fn lookup_tr(en: &str) -> Option<&'static str> {
    Some(match en {
        // Navigation, sidebar & routes (official YouTube TR terms)
        "Home" => "Ana Sayfa",
        "Shorts" => "Shorts",
        "Subscriptions" => "Abonelikler",
        "You" => "Siz",
        "History" => "Geçmiş",
        "Playlists" => "Oynatma listeleri",
        "Playlist" => "Oynatma listesi",
        "Watch later" => "Daha sonra izle",
        "Favorites" => "Favoriler",
        "Liked videos" => "Beğenilen videolar",
        "Downloads" => "İndirilenler",
        "Manage" => "Yönet",
        "Show fewer" => "Daha az göster",
        "Show more" => "Daha fazla göster",
        "Show less" => "Daha az göster",
        "...more" => "...daha fazla",
        "Read more" => "Devamını oku",
        "View all" => "Tümünü gör",
        "Explore" => "Keşfet",
        "Music" => "Müzik",
        "Gaming" => "Oyun",
        "News" => "Haberler",
        "Sports" => "Spor",
        "Live" => "Canlı",
        "LIVE" => "CANLI",
        "UPCOMING" => "YAKINDA",
        "● LIVE" => "● CANLI",
        "Fashion & Beauty" => "Moda ve Güzellik",
        "Learning" => "Eğitim",
        "Trending" => "Trendler",
        "Movies" => "Filmler",
        "Movie" => "Film",
        "Podcasts" => "Podcast'ler",
        "Courses" => "Kurslar",
        "Shows" => "Programlar",
        "Videos" => "Videolar",
        "Video" => "Video",
        "Channels" => "Kanallar",
        "Channel" => "Kanal",
        "Watch" => "İzle",
        "Settings" => "Ayarlar",
        "Your channel" => "Kanalınız",

        // Common buttons, menus & actions
        "Search" => "Ara",
        "Search YouTube" => "Ara",
        "Search or paste a YouTube link" => "Ara veya YouTube bağlantısı yapıştır",
        "Search settings" => "Ayarlarda ara",
        "Search history" => "Arama geçmişi",
        "Search channel" => "Kanalda ara",
        "Search in playlist" => "Oynatma listesinde ara",
        "Filter channels" => "Kanalları filtrele",
        "Subscribe" => "Abone ol",
        "Subscribed" => "Abone olundu",
        "Unsubscribe" => "Abonelikten çık",
        "Like" => "Beğen",
        "Dislike" => "Beğenme",
        "Share" => "Paylaş",
        "Save" => "Kaydet",
        "Saved" => "Kaydedildi",
        "Download" => "İndir",
        "Sign in" => "Oturum aç",
        "Sign out" => "Oturumu kapat",
        "Sign in to YouTube" => "YouTube'da oturum aç",
        "Sign in to chat" => "Sohbete katılmak için oturum açın",
        "Connect" => "Bağlan",
        "Disconnect" => "Bağlantıyı kes",
        "Open" => "Aç",
        "Close" => "Kapat",
        "Cancel" => "İptal",
        "Create" => "Oluştur",
        "Delete" => "Sil",
        "Delete file" => "Dosyayı sil",
        "Clear" => "Temizle",
        "Copy" => "Kopyala",
        "Copy link" => "Bağlantıyı kopyala",
        "Copy all text" => "Tüm metni kopyala",
        "Copy channel link" => "Kanal bağlantısını kopyala",
        "Open on YouTube" => "YouTube'da aç",
        "Open in new tab" => "Yeni sekmede aç",
        "Opened in a new tab" => "Yeni sekmede açıldı",
        "Open in external player" => "Harici oynatıcıda aç",
        "Add to queue" => "Sıraya ekle",
        "Added to queue" => "Sıraya eklendi",
        "Save to Watch later" => "Daha sonra izle'ye kaydet",
        "Saved to Watch later" => "Daha sonra izle'ye kaydedildi",
        "Save to playlist…" => "Oynatma listesine kaydet…",
        "Don't recommend channel" => "Kanalı önerme",
        "Remove from history" => "Geçmişten kaldır",
        "Removed from watch history" => "İzleme geçmişinden kaldırıldı",
        "Removed from library" => "Kitaplıktan kaldırıldı",
        "Saved to library" => "Kitaplığa kaydedildi",
        "View full playlist" => "Oynatma listesinin tamamını görüntüle",
        "Play all" => "Tümünü oynat",
        "Shuffle" => "Karıştır",
        "Refresh" => "Yenile",
        "Retry" => "Tekrar dene",
        "Generate" => "Oluştur",
        "Unblock" => "Engeli kaldır",
        "Unblock channel" => "Kanalın engelini kaldır",
        "Block channel (hide everywhere)" => "Kanalı engelle (her yerde gizle)",
        "Reset remembered speed" => "Kayıtlı hızı sıfırla",
        "Update" => "Güncelle",
        "Update now" => "Şimdi güncelle",
        "Check for updates" => "Güncellemeleri denetle",
        "Choose…" => "Seç…",
        "Choose file…" => "Dosya seç…",
        "Export…" => "Dışa aktar…",
        "Reset" => "Sıfırla",
        "Off" => "Kapalı",
        "On" => "Açık",
        "Auto" => "Otomatik",
        "Automatic" => "Otomatik",
        "Default" => "Varsayılan",
        "None" => "Yok",
        "Normal" => "Normal",
        "Loading…" => "Yükleniyor…",
        "Waiting…" => "Bekleniyor…",
        "Queued" => "Sırada",
        "Starting…" => "Başlatılıyor…",
        "Cancelled" => "İptal edildi",
        "Refreshing…" => "Yenileniyor…",
        "Unpacking…" => "Ayıklanıyor…",
        "Checking…" => "Denetleniyor…",
        "Something went wrong" => "Bir sorun oluştu",
        "Link copied" => "Bağlantı kopyalandı",
        "Link copied to clipboard" => "Bağlantı panoya kopyalandı",
        "Private ID copied" => "Özel kimlik kopyalandı",
        "New SponsorBlock ID created" => "Yeni SponsorBlock kimliği oluşturuldu",
        "Search history cleared" => "Arama geçmişi temizlendi",
        "Watch history cleared" => "İzleme geçmişi temizlendi",
        "Subscriptions are up to date" => "Abonelikler güncel",
        "Channel blocked. Manage in Settings → Content filters" => "Kanal engellendi. Ayarlar → İçerik filtreleri bölümünden yönetebilirsiniz",
        "Couldn't update the rating on your YouTube account" => "YouTube hesabınızdaki beğeni güncellenemedi",
        "Couldn't update the subscription on your YouTube account" => "YouTube hesabınızdaki abonelik güncellenemedi",

        // Friendly errors
        "This page isn't available." => "Bu sayfa kullanılamıyor.",
        "YouTube is rate limiting requests. Try again in a minute." => "YouTube istekleri sınırlandırıyor. Bir dakika sonra tekrar deneyin.",
        "Couldn't reach YouTube. Check your connection." => "YouTube'a ulaşılamadı. İnternet bağlantınızı kontrol edin.",
        "This live stream has ended and its recording isn't available." => "Bu canlı yayın sona erdi ve kaydı mevcut değil.",
        "This hasn't started yet." => "Bu yayın henüz başlamadı.",
        "This video is for channel members only." => "Bu video yalnızca kanal üyelerine özeldir.",
        "This video is private." => "Bu video gizlidir.",

        // Download format labels
        "1080p (recommended)" => "1080p (önerilen)",
        "Best available (up to 4K, large files)" => "En yüksek kalite (4K'ya kadar, büyük dosya)",
        "Up to 720p" => "720p'ye kadar",
        "Up to 480p" => "480p'ye kadar",

        // Watch page, Comments & Live Chat
        "Comments" => "Yorumlar",
        "Top" => "En popüler",
        "Newest" => "En yeni",
        "Timed" => "Zamanlı",
        "Pinned" => "Sabitlendi",
        "Show more replies" => "Daha fazla yanıt göster",
        "Show more comments" => "Daha fazla yorum göster",
        "Comments are turned off." => "Yorumlar kapalı.",
        "No comments yet." => "Henüz yorum yok.",
        "Chapters" => "Bölümler",
        "Queue" => "Sıra",
        "Up next" => "Sıradaki",
        "Autoplay" => "Otomatik oynat",
        "Live chat" => "Canlı sohbet",
        "Chat…" => "Sohbet et…",
        "Chat has ended" => "Canlı sohbet sona erdi",
        "Loading chat…" => "Sohbet yükleniyor…",
        "Could not load chat, retrying…" => "Sohbet yüklenemedi, yeniden deneniyor…",

        // Player controls, overlays & menus
        "Subtitles/CC" => "Altyazılar",
        "Audio track" => "Ses parçası",
        "Quality" => "Kalite",
        "Playback speed" => "Oynatma hızı",
        "Original language" => "Orijinal dil",
        "Video's audio language" => "Videonun ses dili",
        "Audio only" => "Yalnızca ses",
        "Audio only (background)" => "Yalnızca ses (arka plan)",
        "Loop" => "Döngü",
        "Loop: on" => "Döngü: açık",
        "Loop: off" => "Döngü: kapalı",
        "Copy video URL" => "Video URL'sini kopyala",
        "Copy video URL at current time" => "Geçerli zamandaki video URL'sini kopyala",
        "Screenshot" => "Ekran görüntüsü",
        "Stats for nerds" => "Meraklısı için istatistikler",
        "Hide stats for nerds" => "İstatistikleri gizle",
        "Play / pause" => "Oynat / duraklat",
        "Jump ahead" => "İleri atla",
        "Jump to highlight" => "Öne çıkan ana atla",
        " (auto-generated)" => " (otomatik oluşturuldu)",
        "Full" => "Tam video",

        // Home, Library, Playlist, Search, Subscriptions, Downloads
        "All" => "Tümü",
        "For you" => "Sizin için",
        "Popular" => "Popüler",
        "Recently uploaded" => "Son yüklenenler",
        "Nothing here yet" => "Burada henüz bir şey yok",
        "No Shorts found" => "Shorts videosu bulunamadı",
        "No results found" => "Sonuç bulunamadı",
        "Try different keywords or remove filters" => "Farklı anahtar kelimeler deneyin veya filtreleri kaldırın",
        "Filters" => "Filtreler",
        "Filters •" => "Filtreler •",
        "Upload date" => "Yükleme tarihi",
        "Last hour" => "Son 1 saat",
        "Today" => "Bugün",
        "Yesterday" => "Dün",
        "This week" => "Bu hafta",
        "This month" => "Bu ay",
        "This year" => "Bu yıl",
        "Older" => "Daha eski",
        "Type" => "Tür",
        "Duration" => "Süre",
        "Under 4 minutes" => "4 dakikadan kısa",
        "4 - 20 minutes" => "4 - 20 dakika",
        "Over 20 minutes" => "20 dakikadan uzun",
        "FEATURES" => "ÖZELLİKLER",
        "Sort by" => "Sıralama ölçütü",
        "Relevance" => "Alaka düzeyi",
        "View count" => "Görüntüleme sayısı",
        "Rating" => "Derecelendirme",
        "YouTube history" => "YouTube geçmişi",
        "On this device" => "Bu cihazda",
        "On this computer" => "Bu bilgisayarda",
        "Searches" => "Aramalar",
        "Watched" => "İzlenenler",
        "Your watch history is empty" => "İzleme geçmişiniz boş",
        "Your YouTube watch history is empty" => "YouTube izleme geçmişiniz boş",
        "No matches" => "Eşleşme bulunamadı",
        "No searches yet" => "Henüz arama yok",
        "Searches made in this app show here" => "Bu uygulamada yapılan aramalar burada görünür",
        "Sign in to see your YouTube watch history" => "YouTube izleme geçmişinizi görmek için oturum açın",
        "Could not load your YouTube history" => "YouTube geçmişiniz yüklenemedi",
        "Clear watch history" => "İzleme geçmişini temizle",
        "Clear all watch history" => "Tüm izleme geçmişini temizle",
        "Clear watch history?" => "İzleme geçmişi temizlensin mi?",
        "All watch history on this device and resume positions will be deleted." => "Bu cihazdaki tüm izleme geçmişi ve kaldığınız yerler silinecek.",
        "Pause watch history" => "İzleme geçmişini duraklat",
        "Turn on watch history" => "İzleme geçmişini aç",
        "Clear search history" => "Arama geçmişini temizle",
        "Local • Private" => "Yerel • Gizli",
        "New playlist" => "Yeni oynatma listesi",
        "Create new playlist" => "Yeni oynatma listesi oluştur",
        "Playlist name" => "Oynatma listesi adı",
        "Rename playlist" => "Oynatma listesini yeniden adlandır",
        "Name" => "Ad",
        "Delete playlist?" => "Oynatma listesi silinsin mi?",
        "This cannot be undone." => "Bu işlem geri alınamaz.",
        "No videos in this playlist yet" => "Bu oynatma listesinde henüz video yok",
        "No videos match" => "Eşleşen video yok",
        "Your YouTube playlists" => "YouTube oynatma listeleriniz",
        "Saved playlists" => "Kaydedilen oynatma listeleri",
        "No playlists on your account yet" => "Hesabınızda henüz oynatma listesi yok",
        "Title" => "Başlık",
        "Longest" => "En uzun",
        "Shortest" => "En kısa",
        "This channel has no content here" => "Bu kanalın burada içeriği yok",
        "No subscriptions yet" => "Henüz abonelik yok",
        "Import subscriptions" => "Abonelikleri içe aktar",
        "All channels" => "Tüm kanallar",
        "Watched hidden" => "İzlenenler gizli",
        "Hide watched" => "İzlenenleri gizle",
        "No videos" => "Video yok",
        "New profile" => "Yeni profil",
        "Profile name (e.g. Music, Tech)" => "Profil adı (örn. Müzik, Teknoloji)",
        "Delete profile" => "Profili sil",
        "Delete profile?" => "Profil silinsin mi?",
        "Channels stay subscribed." => "Kanal abonelikleri korunur.",
        "In profile" => "Profilde",
        "No downloads yet" => "Henüz indirilen yok",
        "Clear list" => "Listeyi temizle",
        "Change folder" => "Klasörü değiştir",
        "Save downloads here" => "İndirilenleri buraya kaydet",
        "Open folder" => "Klasörü aç",
        "Connect your YouTube history" => "YouTube geçmişinizi bağlayın",
        "Use cookies" => "Çerez dosyasını seç",
        "Use a cookies.txt file instead" => "Bunun yerine cookies.txt dosyası kullan",
        "Try again" => "Tekrar dene",
        "Waiting for sign-in…" => "Oturum açma bekleniyor…",
        "Videos you watch in Stream Tube then show in your YouTube history, and your phone and TV resume them where you stopped. The window uses its own empty profile, which is deleted afterwards." => "Burada izlediğiniz videolar YouTube geçmişinizde görünür; telefonunuz ve TV'nizde kaldığınız yerden devam eder. Pencere kendi boş profilini kullanır ve işlem bitince silinir.",
        "1. On any device, go to" => "1. Herhangi bir cihazdan şu adrese gidin:",
        "2. Enter this code and pick your account" => "2. Bu kodu girin ve hesabınızı seçin",
        "Waiting for approval…" => "Onay bekleniyor…",
        "Copy code & open" => "Kodu kopyala ve aç",
        "Getting a code…" => "Kod alınıyor…",
        "Browser sign-in disconnected" => "Tarayıcı oturum bağlantısı kesildi",
        "Sync subscriptions" => "Abonelikleri eşitle",
        "Switch account" => "Hesap değiştir",
        "Channel handle unknown" => "Kanal kullanıcı adı bilinmiyor",
        "Could not open your channel" => "Kanalınız açılamadı",
        "File missing" => "Dosya bulunamadı",
        "Show in folder" => "Klasörde göster",
        "Remove from list" => "Listeden kaldır",
        "Open downloads page" => "İndirilenler sayfasını aç",
        "No update available" => "Güncelleme yok",
        "Pre-release" => "Ön sürüm",
        "New version" => "Yeni sürüm",
        "WHAT'S NEW" => "YENİLİKLER",
        "Installing…" => "Yükleniyor…",
        "Downloading…" => "İndiriliyor…",
        "Stream Tube restarts by itself when it's done." => "İşlem bitince uygulama kendiliğinden yeniden başlar.",
        "Skip this version" => "Bu sürümü atla",
        "Later" => "Daha sonra",
        "Save video to…" => "Videoyu şuraya kaydet…",

        // Settings sections
        "General" => "Genel",
        "Player" => "Oynatıcı",
        "Premium features" => "Premium özellikler",
        "Tweaks" => "İnce ayarlar",
        "SponsorBlock & DeArrow" => "SponsorBlock ve DeArrow",
        "Content filters" => "İçerik filtreleri",
        "Privacy" => "Gizlilik",
        "Backend & network" => "Arka uç ve ağ",
        "Import / export" => "İçe / dışa aktar",
        "About" => "Hakkında",

        // Settings -> General
        "YouTube account" => "YouTube hesabı",
        "Personal recommendations, subscriptions, likes and history" => "Kişisel öneriler, abonelikler, beğeniler ve geçmiş",
        "YouTube watch history" => "YouTube izleme geçmişi",
        "Sign in through your browser so videos you watch here show in your YouTube history and resume on your phone" => "Burada izlediğiniz videoların YouTube geçmişinizde görünmesi ve diğer cihazlarınızda kaldığınız yerden devam etmesi için tarayıcıdan oturum açın",
        "App language" => "Uygulama dili",
        "Interface language of the desktop client" => "Uygulamanın arayüz dili",
        "Brand name" => "Marka adı",
        "Top-left text and window title. Color codes: [#FF0033]You[#FFFFFF]Tube or &cYou&fTube" => "Sol üstteki yazı ve pencere başlığı. Renk kodları: [#FF0033]You[#FFFFFF]Tube veya &cYou&fTube",
        "Brand font" => "Marka yazı tipi",
        "Font used for the top-left brand text" => "Sol üstteki marka adı için yazı tipi",
        "YouTube Condensed" => "YouTube (Dar)",
        "Custom logo & app icon" => "Özel logo ve uygulama ikonu",
        "PNG, SVG, JPG, WebP or ICO file for the top-left logo and taskbar icon" => "Sol üst logo ve pencere/görev çubuğu ikonu için PNG, SVG, JPG, WebP veya ICO dosyası",
        "Theme" => "Tema",
        "Dark" => "Koyu",
        "Light" => "Açık",
        "System" => "Sistem",
        "Accent color" => "Vurgu rengi",
        "Switches, highlights, progress and the seek bar" => "Anahtarlar, vurgular ve ilerleme çubuğu rengi",
        "Start page" => "Başlangıç sayfası",
        "Interface size" => "Arayüz ölçeği",
        "Always show scrollbars" => "Kaydırma çubuklarını her zaman göster",
        "Otherwise they appear when the pointer is over a page. Drag one, or click its track, to scroll" => "Kapalıyken yalnızca fare sayfanın üzerindeyken görünür",
        "Density" => "Görünüm yoğunluğu",
        "Compact" => "Sıkışık",
        "Comfortable" => "Rahat",
        "Thumbnail corners" => "Küçük resim köşeleri",
        "Square" => "Köşeli",
        "Small" => "Hafif",
        "Rounded" => "Yuvarlatılmış",
        "Large" => "Çok yuvarlatılmış",
        "Grid columns" => "Satır başına video sayısı",
        "Play videos on hover" => "Küçük resim önizlemesi",
        "Muted preview when the pointer rests on a thumbnail" => "Fare videonun üzerinde beklerken sessiz önizleme oynatır",
        "Region" => "Konum",
        "Trending, Explore and search results" => "Trendler, Keşfet ve arama sonuçları için bölge",
        "Language" => "YouTube içerik dili",
        "Titles, dates and YouTube's text" => "Video başlıkları, tarihler ve YouTube metinleri",

        // Settings -> Player
        "Default quality" => "Varsayılan video kalitesi",
        "Maximum resolution to start videos at" => "Videoların başlayacağı en yüksek çözünürlük",
        "Preferred codec" => "Tercih edilen kodek",
        "Default speed" => "Varsayılan oynatma hızı",
        "Remember speed per channel" => "Kanal başına hızı hatırla",
        "Autoplay next video" => "Sonraki videoyu otomatik oynat",
        "Resume where you left off" => "Kaldığım yerden devam et",
        "Loop videos by default" => "Videoları varsayılan olarak döngüye al",
        "Audio only mode" => "Yalnızca ses modu",
        "Hardware decoding" => "Donanım hızlandırma",
        "Takes effect after restart" => "Yeniden başlatınca geçerli olur",
        "Captions on by default" => "Altyazıları varsayılan olarak aç",
        "Caption language" => "Altyazı dili",
        "Preferred subtitle track" => "Tercih edilen altyazı parçası",
        "Caption size" => "Altyazı boyutu",
        "Medium" => "Orta",
        "Extra large" => "Çok büyük",
        "Caption font" => "Altyazı yazı tipi",
        "Sans-serif" => "Sans-serif",
        "Serif" => "Serif",
        "Monospace" => "Eşit aralıklı",
        "Casual" => "El yazısı",
        "Caption color" => "Altyazı rengi",
        "Caption opacity" => "Altyazı opaklığı",
        "Caption background" => "Altyazı arka planı",
        "Background color" => "Arka plan rengi",
        "Background opacity" => "Arka plan opaklığı",
        "Character edge" => "Yazı kenar stili",
        "Around the text when there's no background" => "Arka plan kapalıyken metnin çevresindeki stil",
        "Outline and shadow" => "Dış çizgi ve gölge",
        "Outline" => "Dış çizgi",
        "Drop shadow" => "Gölge",
        "Preview" => "Önizleme",
        "Captions look like this" => "Altyazılar böyle görünür",
        "Hide controls after" => "Kontrolleri gizleme süresi",
        "Seek bar preview thumbnails" => "İlerleme çubuğunda önizleme kareleri",
        "Arrow key seek step" => "Ok tuşlarıyla sarma süresi",
        "External player" => "Harici oynatıcı",
        "Leave empty to use VLC or mpv when installed, or your default player. Or enter the path to another player" => "Yüklüyse VLC/mpv veya sistemin varsayılan oynatıcısı için boş bırakın; başka bir oynatıcı yolu da girebilirsiniz",
        "White" => "Beyaz",
        "Yellow" => "Sarı",
        "Green" => "Yeşil",
        "Cyan" => "Camgöbeği",
        "Blue" => "Mavi",
        "Magenta" => "Macenta",
        "Red" => "Kırmızı",
        "Black" => "Siyah",

        // Settings -> Premium & Tweaks
        "These are built into Stream Tube, so they work without YouTube Premium and without signing in." => "Bu özellikler yerleşiktir; YouTube Premium veya oturum açma gerektirmez.",
        "Background play" => "Arka planda oynatma",
        "Keep playing when the window is minimized (off: pause, resume on restore)" => "Pencere simge durumuna küçültüldüğünde oynatmaya devam et",
        "Most replayed graph" => "En çok tekrar oynatılan grafiği",
        "Heat line above the seek bar" => "İlerleme çubuğunun üzerindeki yoğunluk çizgisi",
        "Button to skip to the most replayed part" => "En çok izlenen bölüme tek tıkla atlama düğmesi",
        "Enhanced bitrate" => "Geliştirilmiş bit hızı",
        "Picks the highest-bitrate stream YouTube offers for the chosen quality" => "Seçilen çözünürlükte YouTube'un sunduğu en yüksek bit hızlı akışı seçer",
        "Ambient mode" => "Ambiyans modu",
        "Soft glow of the video's colors around the player" => "Oynatıcı çevresinde videonun renklerinden yumuşak ışık efekti",
        "Annotations" => "Ek açıklamalar",
        "Notes and links on older videos, restored from the Annotations Restored archive" => "Eski videolardaki notlar ve bağlantılar (Annotations Restored arşivi)",
        "Force original audio" => "Orijinal ses dilini zorla",
        "Play the original language instead of auto-dubbed tracks" => "Otomatik dublaj yerine videonun orijinal sesini oynat",
        "Shorts: play next automatically" => "Shorts: sonrakine otomatik geç",
        "Move to the next Short when one ends" => "Biri bitince sonraki Short videosunu başlatır",
        "Open Shorts in the regular player" => "Shorts videolarını normal oynatıcıda aç",
        "YouTube-style Shorts layout" => "YouTube tarzı Shorts görünümü",
        "Place channel info outside the video (like YouTube) with customizable controls" => "Kanal bilgilerini YouTube gibi videonun sol dışına alır ve özelleştirilebilir kontroller sunar",
        "Edge navigation buttons" => "Sağ kenar gezinme düğmeleri",
        "Place the up/down Short buttons on the far right edge" => "Yukarı/aşağı Shorts geçiş düğmelerini sağ kenara yaslar",
        "Show Subscribe button" => "Abone ol düğmesini göster",
        "Show Dislike button" => "Beğenme düğmesini göster",
        "Remember last quality" => "Son seçilen kaliteyi hatırla",
        "Use the quality you picked last as the default" => "En son seçtiğiniz kaliteyi sonraki videolarda varsayılan yapar",
        "Double-click for fullscreen" => "Çift tıklamayla tam ekran",
        "Exit fullscreen when a video ends" => "Video bitince tam ekrandan çık",
        "Controls shade" => "Oynatıcı kontrolleri karartması",
        "Gradient behind the player controls" => "Oynatıcı tuşlarının okunabilirliği için arka plan karartma yoğunluğu",
        "Strong" => "Koyu",

        // Settings -> SponsorBlock & DeArrow
        "Enable SponsorBlock" => "SponsorBlock'u etkinleştir",
        "Show skip notifications" => "Atlama bildirimlerini göster",
        "With an “Unskip” button" => "“Geri al” düğmesiyle birlikte",
        "Minimum segment duration (seconds)" => "Minimum bölüm süresi (saniye)",
        "Server" => "Sunucu",
        "Private user ID" => "Özel kullanıcı kimliği",
        "Paste the ID from the SponsorBlock extension or another app, or generate one. Keep it secret." => "SponsorBlock eklentisinden kimliğinizi yapıştırın veya yeni oluşturun. Kimseyle paylaşmayın.",
        "Paste your private user ID" => "Özel kullanıcı kimliğinizi yapıştırın",
        "Auto skip" => "Otomatik atla",
        "Ask" => "Sor",
        "Show in seek bar" => "Çubukta göster",
        "DeArrow titles" => "DeArrow başlıkları",
        "DeArrow thumbnails" => "DeArrow küçük resimleri",
        "Return YouTube Dislike" => "Return YouTube Dislike",
        "Loading your stats…" => "İstatistikleriniz yükleniyor…",
        "Your contributions" => "Katkılarınız",
        "segments submitted" => "bölüm gönderildi",
        "skips by others" => "başkaları tarafından atlandı",
        "of people's time saved" => "kişilerin zamanı kurtarıldı",
        "No submissions yet for this ID. Segments you submit will count here." => "Bu kimlik için henüz gönderim yok. Gönderdiğiniz bölümler burada sayılır.",
        "Your SponsorBlock stats" => "SponsorBlock istatistikleriniz",
        "Sponsor" => "Sponsor",
        "Unpaid/Self Promotion" => "Ücretsiz / Kendi Tanıtımı",
        "Interaction Reminder" => "Etkileşim Hatırlatıcısı (Abone Ol / Beğen)",
        "Intermission/Intro" => "Ara / Giriş Animasyonu",
        "Endcards/Credits" => "Bitiş Ekranı / Jenerik",
        "Preview/Recap" => "Önizleme / Özet",
        "Hook/Greetings" => "Kanca / Selamlama",
        "Music: Non-Music" => "Müzik: Müzik Dışı Bölüm",
        "Filler Tangent" => "Doldurma / Konu Dışı",
        "Highlight" => "Öne Çıkan An",
        "Exclusive Access" => "Özel Erişim",

        // Settings -> Content filters, Subscriptions, Privacy, Network, Downloads, Data, About
        "Hide Shorts" => "Shorts videolarını gizle",
        "Hide live streams" => "Canlı yayınları gizle",
        "Hide upcoming / premieres" => "Yaklaşan / ilk gösterimleri gizle",
        "Hide comments" => "Yorumları gizle",
        "Hide related videos" => "Önerilen videoları gizle",
        "Hide description" => "Açıklamayı gizle",
        "Hide view counts" => "Görüntüleme sayılarını gizle",
        "Hide channel avatars" => "Kanal profil resimlerini gizle",
        "Hide chapters" => "Bölümleri gizle",
        "Hide Explore / trending" => "Keşfet ve Trendler'i gizle",
        "Hide watched videos in feeds" => "Akışlarda izlenen videoları gizle",
        "Blocked keywords" => "Engellenen anahtar kelimeler",
        "Add a keyword and press Enter" => "Anahtar kelime yazıp Enter'a basın",
        "Blocked channels" => "Engellenen kanallar",
        "Feed source" => "Akış kaynağı",
        "Include Shorts in feed" => "Akışa Shorts videolarını dahil et",
        "Extra request per channel" => "Kanal başına ek istek gerektirir",
        "Include live streams in feed" => "Akışa canlı yayınları dahil et",
        "Save watch history" => "İzleme geçmişini kaydet",
        "Kept on this computer, and in your YouTube history when connected in General" => "Bu bilgisayarda saklanır; Genel sekmesinden bağlanırsanız YouTube geçmişinizle de eşitlenir",
        "Save search history" => "Arama geçmişini kaydet",
        "Playback client" => "Oynatma istemcisi",
        "Which YouTube app Stream Tube presents itself as to get video streams. Try another one if videos stop playing or fail to load." => "Video akışlarını alırken uygulamanın hangi YouTube istemcisi gibi davranacağını belirler. Videolar açılmazsa başka birini deneyin.",
        "Auto (recommended)" => "Otomatik (önerilen)",
        "Display server" => "Görüntü sunucusu",
        "On Wayland desktops, X11 runs through XWayland. Choose X11 if the window doesn't appear. Takes effect after restart" => "Wayland masaüstlerinde X11, XWayland üzerinden çalışır. Pencere görünmezse X11'i seçin. Yeniden başlatınca geçerli olur",
        "yt-dlp path" => "yt-dlp yolu",
        "yt-dlp (from PATH)" => "yt-dlp (sistem PATH)",
        "yt-dlp not found — install it for downloads in all qualities and the fallback extractor" => "yt-dlp bulunamadı — tüm kalitelerde indirme ve yedek çözücü için yükleyin",
        "yt-dlp not found" => "yt-dlp bulunamadı",
        "Use IPv4 only" => "Yalnızca IPv4 kullan",
        "Avoids YouTube's “confirm you're not a bot” check on flagged IPv6 networks" => "İşaretli IPv6 ağlarında çıkan “bot olmadığınızı doğrulayın” engelini aşmaya yardımcı olur",
        "Proxy" => "Proxy",
        "Restart required" => "Yeniden başlatma gerekir",
        "Update yt-dlp" => "yt-dlp'yi güncelle",
        "Download folder" => "İndirme klasörü",
        "Default format" => "Varsayılan indirme kalitesi",
        "Needed for downloads above 360p. Stream Tube can fetch it for you" => "360p üzeri indirmeler için gereklidir; uygulama otomatik indirebilir",
        "Installed. Joins the video and audio of downloads above 360p" => "Yüklü. 360p üzeri indirmelerde video ve sesi birleştirir",
        "Needed to join video and audio above 360p. Stream Tube can fetch it for you" => "360p üzeri indirmelerde video ve sesi birleştirmek için gereklidir",
        "Import" => "İçe aktar",
        "Takeout, NewPipe, FreeTube, OPML or Stream Tube backup" => "Takeout, NewPipe, FreeTube, OPML veya Stream Tube yedeği",
        "Export full backup" => "Tam yedeği dışa aktar",
        "Export subscriptions (NewPipe JSON)" => "Abonelikleri dışa aktar (NewPipe JSON)",
        "Export subscriptions (YouTube CSV)" => "Abonelikleri dışa aktar (YouTube CSV)",
        "Export subscriptions (OPML)" => "Abonelikleri dışa aktar (OPML)",
        "For RSS readers" => "RSS okuyucular için",
        "Video engine" => "Video motoru",
        "not installed" => "yüklü değil",
        "Updates" => "Güncellemeler",
        "Pre-releases included" => "Ön sürümler dahil",
        "You have the latest version" => "En güncel sürümü kullanıyorsunuz",
        "Check for updates at startup" => "Başlangıçta güncellemeleri denetle",
        "Releases" => "Sürümler",
        "Changelog and downloads on GitHub" => "GitHub'da sürüm notları ve indirmeler",
        _ => return None,
    })
}
