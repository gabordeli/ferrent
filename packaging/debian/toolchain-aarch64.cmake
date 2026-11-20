set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR aarch64)
set(CMAKE_C_COMPILER aarch64-linux-gnu-gcc)
set(CMAKE_CXX_COMPILER aarch64-linux-gnu-g++)
set(CMAKE_FIND_ROOT_PATH /usr/aarch64-linux-gnu)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
# Boost is header-only here and lives in the architecture-independent /usr/include.
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE BOTH)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

# Debian puts the arm64 libraries in the multiarch dir, which is not inside the
# gcc sysroot, so FindOpenSSL/FindZLIB need the paths spelled out.
set(OPENSSL_ROOT_DIR /usr)
set(OPENSSL_SSL_LIBRARY /usr/lib/aarch64-linux-gnu/libssl.so)
set(OPENSSL_CRYPTO_LIBRARY /usr/lib/aarch64-linux-gnu/libcrypto.so)
set(ZLIB_LIBRARY /usr/lib/aarch64-linux-gnu/libz.so)
