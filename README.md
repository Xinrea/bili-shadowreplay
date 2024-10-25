# BiliBili ShadowReplay

![icon](doc/header.png)

![GitHub Actions Workflow Status](https://img.shields.io/github/actions/workflow/status/xinrea/bili-shadowreplay/main.yml)
![GitHub Release](https://img.shields.io/github/v/release/xinrea/bili-shadowreplay)
![GitHub Downloads (all assets, all releases)](https://img.shields.io/github/downloads/xinrea/bili-shadowreplay/total)

BiliBili ShadowReplay 是一个缓存 B 站直播并进行实时编辑投稿的工具。通过划定时间区间，并编辑简单的必需信息，即可完成直播切片以及投稿，将整个流程压缩到分钟级。同时，也支持对缓存的历史直播进行回放，以及相同的切片编辑投稿处理流程。

> [!NOTE]
> 由于软件在快速开发中，截图说明可能有变动，仅供参考

![rooms](doc/summary.png)

## 总览

![rooms](doc/summary.png)

显示直播缓存的占用以及缓存所在磁盘的使用情况。

## 直播间管理

![clip](doc/rooms.png)

显示当前缓存的直播间列表，在添加前需要在账号页面添加至少一个账号（主账号）用于直播流以及用户信息的获取。
操作菜单包含打开直播流、查看历史记录以及删除等操作。其中历史记录以列表形式展示，可以进行回放以及删除。

![archives](doc/archives.png)

无论是正在进行的直播还是历史录播，都可在预览窗口进行回放，同时也可以进行切片编辑以及投稿。关于预览窗口的相关说明请见 [预览窗口](#预览窗口)。

## 消息管理

![messages](doc/messages.png)

执行的各种操作都会留下消息记录，方便查看过去进行的操作。

## 账号管理

![accounts](doc/accounts.png)

程序需要至少一个账号用于直播流以及用户信息的获取，可以在此页面添加账号。目前添加账号仅支持 B 站手机 App 扫码添加。

你可以添加多个账号，但只有一个账号会被标记为主账号，主账号用于直播流的获取。所有账号都可在切片投稿或是观看直播流发送弹幕时自由选择，详情见 [预览窗口](#预览窗口)。

## 预览窗口

![livewindow](doc/livewindow.png)

预览窗口是一个多功能的窗口，可以用于观看直播流、回放历史录播、编辑切片以及投稿等操作。如果当前播放的是直播流，那么会有实时弹幕观看以及发送弹幕相关的选项。

通过预览窗口的快捷键操作，可以快速选择时间区间，进行切片生成以及投稿。

无论是弹幕发送还是投稿，均可自由选择账号，只要在账号管理中添加了该账号。

## 设置

![settings](doc/settings.png)

在设置页面可以进行一些基本的设置，包括缓存和切片的保存路径，以及相关事件是否显示通知等。

> [!WARNING]
> 缓存目录进行切换时，会有文件复制等操作，如果缓存量较大，可能会耗费较长时间；且在此期间预览功能会暂时失效，需要等待操作完成。缓存切换开始和结束均会在消息管理中有记录。
