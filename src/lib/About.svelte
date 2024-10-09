<script type="ts">
    import { getVersion } from "@tauri-apps/api/app";
    import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
    const appWindow = getCurrentWebviewWindow();
    let version = "";
    getVersion().then((v) => {
        version = v;
        appWindow.setTitle(`BiliBili ShadowReplay - v${version}`);
        console.log(version);
    });
</script>

<div class="p-8 pt-12 h-full overflow-auto">
    当前版本: {version}
</div>
