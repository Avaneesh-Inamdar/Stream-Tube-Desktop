package io.github.shiroksh.streamtube;

import android.app.Activity;
import android.app.Application;
import android.app.Dialog;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.net.Uri;
import android.os.Bundle;
import android.os.Build;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.Window;
import android.view.WindowInsets;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.TextView;
import android.widget.Toast;
import java.util.regex.Pattern;

public final class VideoPlayer {
    private static final Pattern VIDEO_ID = Pattern.compile("[A-Za-z0-9_-]{11}");
    private static Dialog active;

    private VideoPlayer() {}

    public static void open(Activity activity, String id, String title, String channel, boolean russian, boolean shortVideo) {
        if (!VIDEO_ID.matcher(id).matches()) {
            return;
        }
        activity.runOnUiThread(() -> {
            if (activity.isFinishing() || activity.isDestroyed()) {
                return;
            }
            if (active != null) {
                active.dismiss();
            }
            try {
                show(activity, id, title, channel, russian, shortVideo);
            } catch (RuntimeException failure) {
                Toast.makeText(activity, russian ? "Плеер недоступен" : "Player unavailable",
                    Toast.LENGTH_SHORT).show();
            }
        });
    }

    private static void show(Activity activity, String id, String title, String channel, boolean russian, boolean shortVideo) {
        FrameLayout frame = new FrameLayout(activity);
        frame.setBackgroundColor(Color.BLACK);
        LinearLayout screen = new LinearLayout(activity);
        screen.setOrientation(LinearLayout.VERTICAL);
        screen.setBackgroundColor(0xFF141518);
        frame.addView(screen, new FrameLayout.LayoutParams(-1, -1));
        frame.setOnApplyWindowInsetsListener((view, insets) -> {
            if (Build.VERSION.SDK_INT >= 30) {
                android.graphics.Insets bars = insets.getInsets(
                    WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
                screen.setPadding(bars.left, bars.top, bars.right, bars.bottom);
            } else {
                screen.setPadding(insets.getSystemWindowInsetLeft(), insets.getSystemWindowInsetTop(),
                    insets.getSystemWindowInsetRight(), insets.getSystemWindowInsetBottom());
            }
            return insets;
        });


        WebView web = new WebView(activity);
        setupWebView(web);

        PlayerChrome chrome = new PlayerChrome(frame, screen);
        Dialog dialog = new Dialog(activity, android.R.style.Theme_Material_NoActionBar) {
            @Override public void onBackPressed() {
                if (!chrome.hideFullscreen()) {
                    super.onBackPressed();
                }
            }
        };
        Button close = action(activity, "‹");
        close.setTextSize(27);
        close.setContentDescription(russian ? "Назад" : "Back");
        close.setOnClickListener(view -> dialog.dismiss());
        TextView label = new TextView(activity);
        label.setText(shortVideo ? "Shorts" : russian ? "Видео" : "Video");
        label.setTextColor(Color.WHITE);
        label.setTextSize(19);
        label.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        label.setPadding(dp(activity, 14), 0, 0, 0);
        label.setGravity(Gravity.CENTER_VERTICAL);
        LinearLayout toolbar = new LinearLayout(activity);
        toolbar.setPadding(dp(activity, 12), 0, dp(activity, 12), 0);
        toolbar.setGravity(Gravity.CENTER_VERTICAL);
        toolbar.addView(close, new LinearLayout.LayoutParams(dp(activity, 46), dp(activity, 44)));
        toolbar.addView(label, new LinearLayout.LayoutParams(0, -1, 1));
        screen.addView(toolbar, new LinearLayout.LayoutParams(-1, dp(activity, 56)));

        web.setWebChromeClient(chrome);
        int width = activity.getResources().getDisplayMetrics().widthPixels;
        int height = activity.getResources().getDisplayMetrics().heightPixels;
        int playerHeight = shortVideo ? Math.round(height * 0.62f) : width * 9 / 16;
        screen.addView(web, new LinearLayout.LayoutParams(-1, Math.max(dp(activity, 200), playerHeight)));

        TextView heading = new TextView(activity);
        heading.setText(title);
        heading.setTextColor(Color.WHITE);
        heading.setTextSize(18);
        heading.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        heading.setMaxLines(3);
        heading.setPadding(dp(activity, 16), dp(activity, 14), dp(activity, 16), dp(activity, 5));
        screen.addView(heading, new LinearLayout.LayoutParams(-1, -2));
        if (!channel.isEmpty()) {
            TextView byline = new TextView(activity);
            byline.setText(channel);
            byline.setTextColor(0xFFBDBBC7);
            byline.setTextSize(14);
            byline.setPadding(dp(activity, 16), 0, dp(activity, 16), dp(activity, 8));
            screen.addView(byline, new LinearLayout.LayoutParams(-1, -2));
        }

        Button youtube = action(activity, russian ? "Открыть в YouTube" : "Open in YouTube");
        youtube.setOnClickListener(view -> {
            try {
                activity.startActivity(new Intent(Intent.ACTION_VIEW,
                    Uri.parse("https://www.youtube.com/watch?v=" + id)));
            } catch (ActivityNotFoundException unavailable) {
                Toast.makeText(activity, russian ? "Браузер недоступен" : "No browser available",
                    Toast.LENGTH_SHORT).show();
            }
        });
        LinearLayout.LayoutParams openParams = new LinearLayout.LayoutParams(-2, dp(activity, 44));
        openParams.setMargins(dp(activity, 16), dp(activity, 7), dp(activity, 16), dp(activity, 10));
        screen.addView(youtube, openParams);

        dialog.setContentView(frame);
        Application app = activity.getApplication();
        PlayerLifecycle lifecycle = new PlayerLifecycle(activity, dialog);
        dialog.setOnDismissListener(closed -> {
            app.unregisterActivityLifecycleCallbacks(lifecycle);
            chrome.hideFullscreen();
            web.stopLoading();
            screen.removeView(web);
            web.destroy();
            if (active == dialog) {
                active = null;
            }
        });
        dialog.show();
        chrome.window = dialog.getWindow();
        app.registerActivityLifecycleCallbacks(lifecycle);
        active = dialog;
        String origin = "https://" + activity.getPackageName();
        String html = "<!doctype html><html><head><meta name='viewport' "
            + "content='width=device-width,initial-scale=1'><meta name='referrer' "
            + "content='strict-origin-when-cross-origin'><style>html,body{margin:0;width:100%;height:100%;"
            + "background:#000}iframe{width:100%;height:100%;border:0}</style></head><body>"
            + "<iframe title='YouTube' src='https://www.youtube.com/embed/" + id
            + "?playsinline=1' referrerpolicy='strict-origin-when-cross-origin' allow='fullscreen; "
            + "encrypted-media; picture-in-picture' allowfullscreen></iframe></body></html>";
        web.loadDataWithBaseURL(origin, html, "text/html", "UTF-8", null);
    }

    private static void setupWebView(WebView web) {
        web.setBackgroundColor(Color.BLACK);
        WebSettings settings = web.getSettings();
        settings.setJavaScriptEnabled(true);
        settings.setDomStorageEnabled(true);
        settings.setAllowFileAccess(false);
        settings.setAllowContentAccess(false);
        settings.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
        web.setWebViewClient(new WebViewClient() {
            @Override public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest req) {
                Uri uri = req.getUrl();
                return req.isForMainFrame() || !"https".equals(uri.getScheme());
            }
        });
    }

    private static Button action(Activity activity, String title) {
        Button button = new Button(activity);
        button.setText(title);
        button.setAllCaps(false);
        button.setTextColor(Color.WHITE);
        button.setTextSize(13);
        button.setPadding(dp(activity, 13), 0, dp(activity, 13), 0);
        GradientDrawable fill = new GradientDrawable();
        fill.setColor(0xFF272831);
        fill.setCornerRadius(dp(activity, 9));
        button.setBackground(fill);
        return button;
    }

    private static int dp(Activity activity, int value) {
        return Math.round(activity.getResources().getDisplayMetrics().density * value);
    }

    private static final class PlayerLifecycle implements Application.ActivityLifecycleCallbacks {
        private final Activity host;
        private final Dialog dialog;

        PlayerLifecycle(Activity host, Dialog dialog) {
            this.host = host;
            this.dialog = dialog;
        }

        @Override public void onActivityPaused(Activity activity) {
            if (activity == host) {
                dialog.dismiss();
            }
        }
        @Override public void onActivityDestroyed(Activity activity) {
            if (activity == host) {
                dialog.dismiss();
            }
        }
        @Override public void onActivityCreated(Activity activity, Bundle saved) {}
        @Override public void onActivityStarted(Activity activity) {}
        @Override public void onActivityResumed(Activity activity) {}
        @Override public void onActivityStopped(Activity activity) {}
        @Override public void onActivitySaveInstanceState(Activity activity, Bundle out) {}
    }

    private static final class PlayerChrome extends WebChromeClient {
        private final FrameLayout frame;
        private final View screen;
        private View fullscreen;
        private Window window;
        private int normalChrome;
        private CustomViewCallback fullscreenCallback;

        PlayerChrome(FrameLayout frame, View screen) {
            this.frame = frame;
            this.screen = screen;
        }

        @Override public void onShowCustomView(View view, CustomViewCallback callback) {
            if (fullscreen != null) {
                callback.onCustomViewHidden();
                return;
            }
            if (view.getParent() instanceof ViewGroup) {
                ((ViewGroup) view.getParent()).removeView(view);
            }
            fullscreen = view;
            fullscreenCallback = callback;
            screen.setVisibility(View.GONE);
            if (window != null) {
                View decor = window.getDecorView();
                normalChrome = decor.getSystemUiVisibility();
                decor.setSystemUiVisibility(normalChrome | View.SYSTEM_UI_FLAG_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY);
            }
            frame.addView(view, new FrameLayout.LayoutParams(-1, -1));
        }

        @Override public void onHideCustomView() {
            hideFullscreen();
        }

        boolean hideFullscreen() {
            if (fullscreen == null) {
                return false;
            }
            View view = fullscreen;
            CustomViewCallback callback = fullscreenCallback;
            fullscreen = null;
            fullscreenCallback = null;
            frame.removeView(view);
            screen.setVisibility(View.VISIBLE);
            if (window != null) {
                window.getDecorView().setSystemUiVisibility(normalChrome);
            }
            callback.onCustomViewHidden();
            return true;
        }
    }
}
