# 抖音账号真实信息获取功能实现

## 概述

本次更新实现了从抖音API获取真实的用户名称和UID，替代原有的随机生成机制。同时，程序启动时会自动更新抖音账号信息，与B站账号保持一致的行为。

## 主要变更

### 1. 添加抖音用户信息获取API

**文件**: `src-tauri/src/recorder/douyin/client.rs`

新增 `get_user_info` 方法：
- 使用抖音IM关系接口 `https://www.douyin.com/aweme/v1/web/im/spotlight/relation/` 获取用户信息
- 通过 `owner_sec_uid` 确定当前登录用户身份
- 在 `followings` 数组中查找匹配的用户信息
- 返回包含真实UID、昵称和头像的用户信息

**文件**: `src-tauri/src/recorder/douyin/response.rs`

新增抖音关系API响应结构体：
- `DouyinRelationResponse` - 主响应结构
- `Following` - 关注用户信息结构
- `Extra2`, `LogPb`, `AvatarSmall` - 辅助结构体

### 3. 数据库支持UID更新

**文件**: `src-tauri/src/database/account.rs`

新增 `update_account_with_uid` 方法：
- 支持更新包括UID在内的账号信息
- 当UID发生变化时，自动删除旧记录并创建新记录
- 确保数据库的一致性

### 4. 账号添加时获取真实信息

**文件**: `src-tauri/src/handlers/account.rs`

修改 `add_account` 函数：
- 抖音账号添加后立即调用API获取真实用户信息
- 自动更新账号的UID、昵称和头像
- 失败时保留账号但使用默认值，并记录警告日志

### 5. 程序启动时自动更新账号信息

**文件**: `src-tauri/src/main.rs`

修改 `setup_app_state` 和 `setup_server_state` 函数：
- 程序启动时遍历所有抖音账号
- 自动调用API更新账号信息
- 支持GUI模式和headless模式

### 6. 前端界面优化

**文件**: `src/page/Account.svelte`

- 抖音账号现在显示真实的用户昵称
- 统一了B站和抖音账号的显示格式
- 显示UID信息和平台标识

## 使用场景

### 添加新账号
1. 用户添加抖音账号Cookie
2. 系统自动从抖音API获取真实的用户信息
3. 数据库存储真实的UID和昵称
4. 前端界面显示真实的用户名

### 程序启动
1. 程序启动时扫描所有账号
2. 对抖音账号调用API更新信息
3. 确保账号信息的时效性
4. 记录更新结果到日志

## 技术细节

### API调用策略
1. **主要方式**: 使用抖音IM关系接口 `https://www.douyin.com/aweme/v1/web/im/spotlight/relation/`
2. **数据获取**: 从响应中的 `owner_sec_uid` 和 `followings` 数组获取用户信息
3. **备用机制**: 如果在关注列表中找不到自己，使用 `owner_sec_uid` 创建基本用户信息
4. **错误处理**: 失败时保留现有账号信息并记录日志

### 新API优势
- **更可靠**: 直接使用抖音官方IM接口，避免HTML解析的不稳定性
- **更完整**: 能够获取完整的用户信息，包括头像、昵称等
- **更准确**: 通过 `owner_sec_uid` 精确匹配当前登录用户
- **结构化数据**: JSON响应便于解析和维护

### 数据库处理
- 使用事务确保数据一致性
- 支持UID变更时的记录替换
- 保持原有Cookie和其他配置信息

### 错误恢复
- API调用失败不影响账号的基本功能
- 记录详细的错误日志便于调试
- 支持后续重试机制

## 兼容性

- 完全向后兼容现有的抖音账号
- 不影响B站账号的现有功能
- 支持GUI和headless两种运行模式

## 测试

添加了单元测试验证JSON解析逻辑：
- `test_douyin_relation_response_parsing` - 测试新的关系API响应结构解析
- `test_douyin_user_info_parsing` - 测试用户信息结构体的反序列化
- 验证关键字段的正确解析，包括 `owner_sec_uid` 和 `followings` 数组
- 确保API响应处理的稳定性
