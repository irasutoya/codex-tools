use tauri::WebviewWindow;
use webview2_com::{
    ClearBrowsingDataCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_BROWSING_DATA_KINDS, COREWEBVIEW2_BROWSING_DATA_KINDS_GENERAL_AUTOFILL,
        COREWEBVIEW2_BROWSING_DATA_KINDS_PASSWORD_AUTOSAVE, ICoreWebView2_13,
        ICoreWebView2Profile2, ICoreWebView2Profile6,
    },
};
use windows::core::Interface;

pub fn configure(window: &WebviewWindow) -> tauri::Result<()> {
    window.with_webview(|platform_webview| {
        if let Err(error) = configure_profile(platform_webview) {
            eprintln!("无法配置 WebView2 输入隐私设置: {error}");
        }
    })
}

fn configure_profile(
    platform_webview: tauri::webview::PlatformWebview,
) -> windows::core::Result<()> {
    unsafe {
        let webview = platform_webview.controller().CoreWebView2()?;
        let profile = webview.cast::<ICoreWebView2_13>()?.Profile()?;
        let profile6 = profile.cast::<ICoreWebView2Profile6>()?;
        profile6.SetIsGeneralAutofillEnabled(false)?;
        profile6.SetIsPasswordAutosaveEnabled(false)?;

        let profile2 = profile.cast::<ICoreWebView2Profile2>()?;
        let data_kinds = COREWEBVIEW2_BROWSING_DATA_KINDS(
            COREWEBVIEW2_BROWSING_DATA_KINDS_GENERAL_AUTOFILL.0
                | COREWEBVIEW2_BROWSING_DATA_KINDS_PASSWORD_AUTOSAVE.0,
        );
        profile2.ClearBrowsingData(
            data_kinds,
            &ClearBrowsingDataCompletedHandler::create(Box::new(move |result| {
                if let Err(error) = result {
                    eprintln!("无法清除 WebView2 自动填充记录: {error}");
                }
                Ok(())
            })),
        )?;
    }
    Ok(())
}
