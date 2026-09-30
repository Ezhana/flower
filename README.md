# Flower



## 仓库结构



## 开发

项目使用 `cargo xtask` 管理 C/C++ native demo 的构建流程。这里的构建并不是单条 Cargo 命令，而是一个多步骤 workflow：

```text
Cargo
  ↓
构建 Rust cdylib
  ↓
生成平台动态库
  ↓
gcc / g++ / clang / clang++
  ↓
链接 C/C++ demo
```

`.cargo/config.toml`：

```toml
[alias]
xtask = "run --package xtask --"
```

### 常用命令

```bash
# 只构建 Rust 动态库
cargo xtask build

# 构建 Rust + C demo
cargo xtask build-c

# 构建 Rust + C++ demo
cargo xtask build-cpp

# 构建 Rust + C + C++ demo
cargo xtask build-all

# 构建并运行 C demo
cargo xtask run-c

# 构建并运行 C++ demo
cargo xtask run-cpp

# 清理 C/C++ demo 产物
cargo xtask clean-demo
```

生成的 native executable 位于：

```text
target/ffi-demo/
├── c_demo
└── cpp_demo
```

### Compiler selection

Linux：

```text
gcc
g++
```

macOS：

```text
clang
clang++
```

可以通过环境变量覆盖：

```bash
CC=clang cargo xtask build-c
CXX=clang++ cargo xtask build-cpp
```

Windows(MSVC)：

```bat
cargo xtask build-c
cargo xtask build-cpp
```


## 文档
