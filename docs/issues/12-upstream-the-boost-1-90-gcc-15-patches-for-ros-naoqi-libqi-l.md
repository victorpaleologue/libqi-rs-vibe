---
title: Upstream the Boost 1.90 / GCC 15 patches for ros-naoqi libqi, libqicore and naoqi_driver2
labels: documentation
---

`contrib/ros-naoqi/` holds three patches needed to build the ROS 2 driver stack from source in a RoboStack Jazzy environment shipping Boost 1.90, GCC 15 and CMake 4:

| Patch | Repository | Content |
|---|---|---|
| `naoqi_libqi.patch` | `ros-naoqi/libqi` (branch `ros2`) | Boost.Asio 1.87+ removals (`io_service`, resolver queries, `wrap`), variadic Boost.Bind placeholders, Boost.Process v1 header path, Boost.Filesystem and Boost.Uuid renames, a GCC 15 template-body fix |
| `naoqi_libqicore.patch` | `ros-naoqi/libqicore` (branch `ros2`) | C++14 for Boost.Lockfree |
| `naoqi_driver.patch` | `ros-naoqi/naoqi_driver2` | drop the header-only `system` Boost component from `find_package` |

They exist only to reproduce `docs/validation.md`. Upstreaming them (or finding equivalents upstream) lets us drop the directory. The Asio changes need the most review since they must keep building with the Boost versions of the Humble and Jazzy binaries.
