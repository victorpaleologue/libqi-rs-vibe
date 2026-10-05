# Building naoqi_driver2 against recent Boost

Patches used to build the ROS 2 driver of the robots and its `libqi` fork from source in a
RoboStack Jazzy environment shipping Boost 1.90, GCC 15 and CMake 4, for the validation
described in `docs/validation.md`. They are not needed on distributions whose Boost predates
1.87.

| Patch | Repository | Content |
|---|---|---|
| `naoqi_libqi.patch` | `ros-naoqi/libqi` (branch `ros2`) | Boost.Asio 1.87+ API removals (`io_service`, resolver queries, `wrap`), Boost.Bind variadic lists, Boost.Process v1, Boost.Filesystem and Boost.Uuid renames, a GCC 15 template-body fix |
| `naoqi_libqicore.patch` | `ros-naoqi/libqicore` (branch `ros2`) | C++14 for Boost.Lockfree |
| `naoqi_driver.patch` | `ros-naoqi/naoqi_driver2` | drop the header-only `system` Boost component |

```sh
cd ros_ws/src
git -C naoqi_libqi apply path/to/naoqi_libqi.patch
git -C naoqi_libqicore apply path/to/naoqi_libqicore.patch
git -C naoqi_driver apply path/to/naoqi_driver.patch
cd .. && colcon build --merge-install --symlink-install --packages-up-to naoqi_driver
```
