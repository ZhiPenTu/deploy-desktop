# 镜像清单规范 deploy.desktop/v1

清单用于描述一组需要从镜像仓库拉取的镜像，以及可选的部署产物。支持 YAML 与 JSON，最大 5 MB。

## 顶层结构

```yaml
apiVersion: deploy.desktop/v1
kind: ImageBundle
metadata:
  name: demo-stack
  version: 1.2.0
  description: 示例镜像清单
  product: Demo Product
  channel: stable
  createdAt: 2026-10-19T00:00:00Z
  minToolVersion: 0.1.0
spec:
  registry: registry.cn-hangzhou.aliyuncs.com
  namespace: demo
  platform: linux/amd64
  images: []
  artifacts: []
  checksums: sha256sum.txt
  signature: cosign.bundle
```

## 字段说明

### metadata

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `name` | 是 | 清单名称，不能为空 |
| `version` | 是 | 清单版本，不能为空 |
| `description` | 否 | 描述 |
| `product` | 否 | 产品名称，用于界面展示 |
| `channel` | 否 | 发布通道，例如 stable / beta |
| `createdAt` | 否 | 创建时间，ISO 8601 |
| `minToolVersion` | 否 | 要求的最低工具版本；低于该版本时界面给出警告 |

### spec

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `registry` | 是 | 仓库地址，例如 `registry.cn-hangzhou.aliyuncs.com` |
| `namespace` | 否 | 仓库命名空间，可包含多级路径 |
| `platform` | 否 | 默认平台，格式为 `os/arch`，例如 `linux/amd64` |
| `images` | 是 | 镜像列表，不能为空 |
| `artifacts` | 否 | 额外产物列表 |
| `checksums` | 否 | 校验和文件路径或 URL |
| `signature` | 否 | 签名文件路径或 URL |

### spec.images

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `name` | 是 | 清单内唯一的镜像名称 |
| `repository` | 是 | 相对 `namespace` 的仓库路径，不允许 `..`、绝对路径或 `@` |
| `digest` | 是 | `sha256:` 加 64 位小写十六进制 |
| `tag` | 否 | 可选的 tag，仅作展示，实际引用使用 digest |
| `sizeBytes` | 否 | 镜像大小，用于预估总体积 |
| `platform` | 否 | 覆盖默认平台 |

最终镜像引用为：

```text
<registry>/<namespace>/<repository>@<digest>
```

### spec.artifacts

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `name` | 是 | 产物名称 |
| `path` | 否 | 相对路径 |
| `url` | 否 | 下载地址 |
| `sha256` | 否 | 产物摘要 |
| `sizeBytes` | 否 | 产物大小 |

## 校验规则

加载清单时会执行以下校验，任一失败即拒绝加载：

- `apiVersion` 必须为 `deploy.desktop/v1`
- `kind` 必须为 `ImageBundle`
- `metadata.name`、`metadata.version` 不能为空
- `spec.images` 不能为空，镜像名称不能重复
- `spec.registry` 不能包含路径、`@` 或非法字符
- `namespace` 与 `repository` 不能包含 `..`、`//`、`@`、`:`
- 每个镜像必须提供合法 `sha256` digest
- `platform` 必须是 `os/arch` 形式
- 未声明的字段会被拒绝，避免拼写错误被静默忽略

## 拉取行为

1. 可选从已保存镜像源替换仓库地址和命名空间，重新预览实际镜像引用；版本摘要与平台不变。
2. 如填写仓库用户名或选择有凭据的镜像源，使用临时 Docker 配置执行 `docker login --password-stdin`。
3. 对每个选中镜像执行 `docker pull [--platform <platform>] <reference>`。
4. 执行 `docker image inspect` 校验 `RepoDigests` 是否包含清单中的 digest。
5. 如填写导出目录，执行 `docker save -o <dir>/<name>.tar <reference>`。

远程模式通过 SSH 在目标机执行以上命令；本地模式在当前机器执行。

镜像源配置、密码与测试范围见 [镜像源管理](registry-sources.md)。
