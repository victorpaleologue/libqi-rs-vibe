#!/usr/bin/env bash
# Runs naoqi_driver2 (ROS 2 Jazzy) against naoqi-sim and checks what reaches ROS.
set +u
# Environment: a sourced ROS 2 workspace with naoqi_driver built (see docs/validation.md).
if [ -n "${MAMBA_ROOT_PREFIX:-}" ] && [ -n "${ROS_ENV:-}" ]; then
  eval "$(micromamba shell hook -s bash)"
  micromamba activate "$ROS_ENV"
fi
source "${ROS_WS:-$HOME/ros_ws}/install/setup.bash"
export ROS_DOMAIN_ID=${ROS_DOMAIN_ID:-42}
export RMW_IMPLEMENTATION=${RMW_IMPLEMENTATION:-rmw_fastrtps_cpp}

OUT=${OUT:-/tmp/naoqi-sim-validate}; mkdir -p "$OUT"
SIM=${NAOQI_SIM:-$(dirname "$0")/../../target/debug/naoqi-sim}
QICLI="${QI_CLI:-$(dirname "$0")/../../target/debug/qi-cli} --url tcp://127.0.0.1:9559"
ROBOT=${ROBOT:-nao}
rm -f "$OUT"/*.txt "$OUT"/*.log

cleanup() { kill "$DRV_PID" "$SIM_PID" 2>/dev/null; wait 2>/dev/null; }
trap cleanup EXIT

"$SIM" --robot "$ROBOT" --listen tcp://127.0.0.1:9559 -v > "$OUT/sim.log" 2>&1 &
SIM_PID=$!
sleep 1.5
ros2 run naoqi_driver naoqi_driver_node --ros-args -p nao_ip:=127.0.0.1 -p nao_port:=9559 \
  -p qi_listen_url:=tcp://127.0.0.1:0 > "$OUT/driver.log" 2>&1 &
DRV_PID=$!
sleep 10
ros2 topic list > "$OUT/topics.txt" 2>&1
echo "== topics =="; cat "$OUT/topics.txt"

PASS=0; FAIL=0
check() { local name=$1; shift; if timeout 20 "$@" > "$OUT/$name.txt" 2>&1; then echo "PASS $name"; PASS=$((PASS+1)); else echo "FAIL $name (see $OUT/$name.txt)"; FAIL=$((FAIL+1)); fi; }
topic() { grep -m1 -E "$1" "$OUT/topics.txt"; }
echo_once() { local name=$1 pattern=$2; local t; t=$(topic "$pattern"); if [ -z "$t" ]; then echo "FAIL $name (no topic matching $pattern)"; FAIL=$((FAIL+1)); return; fi; check "$name" ros2 topic echo --once --no-arr "$t"; }

echo_once joint_states 'joint_states$'
echo_once info '/info$'
echo_once imu 'imu/torso$'
echo_once sonar 'sonar/(left|front)$'
echo_once diagnostics 'diagnostics$'
echo_once camera_front 'camera/front/image_raw$'
echo_once camera_bottom 'camera/bottom/image_raw$'
echo_once odom '/odom$'
echo_once tf '/tf$'
echo_once audio '/audio$'

# Touch: raise a bumper event on the simulator while a subscriber waits.
BUMPER=$(topic 'bumper$')
if [ -n "$BUMPER" ]; then
  ( sleep 3; $QICLI call ALMemory.raiseEvent LeftBumperPressed 1.0 > "$OUT/raise-bumper.txt" 2>&1 ) &
  check bumper ros2 topic echo --once "$BUMPER"
fi

# Speech: publish on /speech, then check the simulator said it.
SPEECH=$(topic 'speech$')
if [ -n "$SPEECH" ]; then
  ros2 topic pub --once "$SPEECH" std_msgs/msg/String "{data: 'hello from ros'}" > "$OUT/pub-speech.txt" 2>&1
  sleep 2
  if $QICLI call ALMemory.getData ALTextToSpeech/CurrentSentence 2>&1 | tee "$OUT/speech.txt" | grep -q "hello from ros"; then echo "PASS speech"; PASS=$((PASS+1)); else echo "FAIL speech"; FAIL=$((FAIL+1)); fi
fi

# Joint command: publish joint angles, then read them back from ALMotion.
JOINT_ANGLES=$(topic 'joint_angles$')
if [ -n "$JOINT_ANGLES" ]; then
  ros2 topic pub --once "$JOINT_ANGLES" naoqi_bridge_msgs/msg/JointAnglesWithSpeed \
    "{joint_names: ['HeadYaw'], joint_angles: [0.5], speed: 1.0, relative: 0}" > "$OUT/pub-joint-angles.txt" 2>&1
  sleep 3
  if $QICLI call ALMotion.getAngles '["HeadYaw"]' true 2>&1 | tee "$OUT/joint-angles.txt" | grep -qE "0\.(4[5-9]|5)"; then echo "PASS joint_angles"; PASS=$((PASS+1)); else echo "FAIL joint_angles"; FAIL=$((FAIL+1)); fi
fi

# Base velocity: publish a twist, then read the robot velocity back.
CMD_VEL=$(topic 'cmd_vel$')
if [ -n "$CMD_VEL" ]; then
  ros2 topic pub --once "$CMD_VEL" geometry_msgs/msg/Twist "{linear: {x: 0.2}, angular: {z: 0.1}}" > "$OUT/pub-cmd-vel.txt" 2>&1
  sleep 2
  if $QICLI call ALMotion.getRobotVelocity 2>&1 | tee "$OUT/robot-velocity.txt" | grep -qvE "^\[?0(\.0+)?,"; then echo "PASS cmd_vel"; PASS=$((PASS+1)); else echo "FAIL cmd_vel"; FAIL=$((FAIL+1)); fi
fi

echo "== summary: $PASS passed, $FAIL failed =="
echo "== driver log tail =="; tail -20 "$OUT/driver.log"
echo "== sim log (warnings and errors) =="; grep -iE "warn|error" "$OUT/sim.log" | head -20
