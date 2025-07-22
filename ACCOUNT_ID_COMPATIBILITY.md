# 账号ID兼容性修改总结

## 概述

为了支持抖音账号的sec_uid（字符串）和B站账号的uid（数字），在保持向后兼容性的前提下，添加了新的id_str字段。

## 主要变更

### 1. 数据库结构修改

**Migration**: 添加了版本5的migration，在accounts表中新增`id_str`列
```sql
ALTER TABLE accounts ADD COLUMN id_str TEXT;
```

**AccountRow结构体**:
```rust
pub struct AccountRow {
    pub platform: String,
    pub uid: u64, // 保持B站兼容性
    pub id_str: Option<String>, // 新增：支持抖音sec_uid
    pub name: String,
    pub avatar: String,
    pub csrf: String,
    pub cookies: String,
    pub created_at: String,
}
```

### 2. 数据库方法更新

**新增方法**:
- `remove_account_by_id_str()` - 通过id_str删除账号
- `update_account_by_id_str()` - 通过id_str更新账号信息
- `update_account_with_id_str()` - 更新账号的id_str和信息
- `get_account_by_id_str()` - 通过id_str查找账号

**保持的方法**: 所有原有方法保持不变，确保向后兼容

### 3. 账号创建逻辑

**B站账号**:
- `uid`: 从Cookie中解析的数字ID
- `id_str`: `None`

**抖音账号**:
- `uid`: 临时生成的数字ID（用于数据库约束）
- `id_str`: 临时字符串（后续更新为真实的sec_uid）

### 4. 账号更新逻辑

**B站账号**: 使用`update_account()`方法，通过uid更新
**抖音账号**: 使用`update_account_with_id_str()`方法，通过id_str更新

### 5. 前端显示

**B站账号**: 显示 "UID: {uid}"
**抖音账号**: 显示 "ID: {id_str || uid}"

## 兼容性保证

✅ **向后兼容**: 现有B站账号无需任何修改
✅ **数据完整性**: 保持原有uid主键约束
✅ **API兼容**: 所有原有API方法保持不变
✅ **前端兼容**: AccountItem接口添加可选的id_str字段

## 使用场景

### B站账号
- 创建: 解析Cookie获取数字uid
- 标识: 使用uid字段
- 显示: UID: 123456789

### 抖音账号
- 创建: 生成临时uid，后续更新id_str为sec_uid
- 标识: 使用id_str字段（sec_uid）
- 显示: ID: MS4wLjABAAAA...

## 数据库查询示例

```rust
// B站账号 - 使用uid
let bili_account = db.get_account("bilibili", 123456789).await?;

// 抖音账号 - 使用id_str
let douyin_account = db.get_account_by_id_str("douyin", "MS4wLjABAAAA...").await?;
```

这种设计既保持了现有B站账号的完全兼容性，又为抖音账号提供了正确的字符串ID支持。
