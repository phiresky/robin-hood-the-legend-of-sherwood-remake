package io.github.phiresky.robinhood;

public class RobinHoodActivity extends android.app.NativeActivity {
    static {
        System.loadLibrary("robin_rs");
    }

    private static native void nativeOnBackPressed();

    private boolean backCallbackRegistered = false;

    private final android.window.OnBackInvokedCallback backCallback =
            new android.window.OnBackInvokedCallback() {
                @Override
                public void onBackInvoked() {
                    android.util.Log.i("RobinHoodActivity", "onBackInvoked");
                    nativeOnBackPressed();
                }
            };

    @Override
    protected void onCreate(android.os.Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        android.util.Log.i("RobinHoodActivity", "onCreate");
        configureSafeFullscreen();
        registerBackCallback();
    }

    @Override
    protected void onResume() {
        super.onResume();
        android.util.Log.i("RobinHoodActivity", "onResume");
        configureSafeFullscreen();
        getWindow().getDecorView().post(new Runnable() {
            @Override
            public void run() {
                registerBackCallback();
            }
        });
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            // System bars may reappear after an activity transition or IME.
            configureSafeFullscreen();
        }
    }

    /**
     * Use the whole safe landscape content area while keeping the game canvas
     * out of display cutouts. Gesture/navigation bars are transient overlays;
     * winit receives the resulting content resize and reflows the logical HUD.
     */
    private void configureSafeFullscreen() {
        final android.view.Window window = getWindow();
        if (android.os.Build.VERSION.SDK_INT >= 28) {
            final android.view.WindowManager.LayoutParams attrs = window.getAttributes();
            attrs.layoutInDisplayCutoutMode =
                    android.view.WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_NEVER;
            window.setAttributes(attrs);
        }
        if (android.os.Build.VERSION.SDK_INT >= 30) {
            window.setDecorFitsSystemWindows(true);
            final android.view.WindowInsetsController controller = window.getInsetsController();
            if (controller != null) {
                controller.hide(android.view.WindowInsets.Type.statusBars()
                        | android.view.WindowInsets.Type.navigationBars());
                controller.setSystemBarsBehavior(
                        android.view.WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
        } else {
            window.getDecorView().setSystemUiVisibility(
                    android.view.View.SYSTEM_UI_FLAG_FULLSCREEN
                            | android.view.View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                            | android.view.View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY);
        }
    }

    private void registerBackCallback() {
        if (android.os.Build.VERSION.SDK_INT >= 33) {
            if (backCallbackRegistered) {
                getOnBackInvokedDispatcher().unregisterOnBackInvokedCallback(backCallback);
                backCallbackRegistered = false;
            }
            getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    android.window.OnBackInvokedDispatcher.PRIORITY_OVERLAY,
                    backCallback);
            backCallbackRegistered = true;
            android.util.Log.i("RobinHoodActivity", "registered back callback");
        }
    }

    @Override
    public boolean dispatchKeyEvent(android.view.KeyEvent event) {
        if (event.getKeyCode() == android.view.KeyEvent.KEYCODE_BACK
                && event.getAction() == android.view.KeyEvent.ACTION_UP) {
            android.util.Log.i("RobinHoodActivity", "dispatchKeyEvent BACK");
            nativeOnBackPressed();
            return true;
        }
        return super.dispatchKeyEvent(event);
    }

    @Override
    public void onBackPressed() {
        android.util.Log.i("RobinHoodActivity", "onBackPressed");
        nativeOnBackPressed();
    }

    public void finishFromNative(int exitCode) {
        android.util.Log.i("RobinHoodActivity", "finishFromNative exitCode=" + exitCode);
        if (android.os.Build.VERSION.SDK_INT >= 21) {
            finishAndRemoveTask();
        } else {
            finish();
        }
    }
}
