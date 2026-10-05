// A libqi C++ client replaying the NAOqi calls of naoqi_driver2 against naoqi-sim, with the
// exact call forms and value accessors of the driver (see interop/README.md).
//
// Usage: driver_smoke tcp://127.0.0.1:9559
// Prints one "OK <step>" line per step and "ALL OK" at the end; exits with 1 on failure.

#include <qi/anyobject.hpp>
#include <qi/application.hpp>
#include <qi/session.hpp>
#include <qi/anyvalue.hpp>
#include <qicore/logmanager.hpp>
#include <qicore/loglistener.hpp>
#include <qicore/logmessage.hpp>

#include <boost/algorithm/string.hpp>
#include <boost/make_shared.hpp>
#include <atomic>
#include <chrono>
#include <cmath>
#include <iostream>
#include <map>
#include <mutex>
#include <sstream>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

namespace {

struct Failure : std::runtime_error {
  using std::runtime_error::runtime_error;
};

void check(bool condition, const std::string& what) {
  if (!condition) throw Failure(what);
}

void ok(const std::string& step) { std::cout << "OK " << step << std::endl; }

template <typename Predicate>
void waitFor(Predicate predicate, const std::string& what, int timeoutMs = 5000) {
  for (int elapsed = 0; elapsed < timeoutMs; elapsed += 20) {
    if (predicate()) return;
    std::this_thread::sleep_for(std::chrono::milliseconds(20));
  }
  throw Failure("timed out waiting for " + what);
}

// naoqi_driver2 src/tools/from_any_value.cpp, verbatim accessors.
struct NaoqiImage {
  int width, height, number_of_layers, colorspace, timestamp_s, timestamp_us;
  void* buffer;
  int cam_id;
  float fov_left, fov_top, fov_right, fov_bottom;
};

NaoqiImage fromAnyValueToNaoqiImage(qi::AnyValue& value) {
  qi::AnyReferenceVector anyref = value.asListValuePtr();
  NaoqiImage result;
  auto intAt = [&](size_t i, const char* name) {
    qi::AnyReference ref = anyref[i].content();
    if (ref.kind() != qi::TypeKind_Int) throw Failure(std::string("Could not retrieve ") + name);
    return ref.asInt32();
  };
  auto floatAt = [&](size_t i, const char* name) {
    qi::AnyReference ref = anyref[i].content();
    if (ref.kind() != qi::TypeKind_Float) throw Failure(std::string("Could not retrieve ") + name);
    return ref.asFloat();
  };
  result.width = intAt(0, "width");
  result.height = intAt(1, "height");
  result.number_of_layers = intAt(2, "number of layers");
  result.colorspace = intAt(3, "colorspace");
  result.timestamp_s = intAt(4, "timestamp_s");
  result.timestamp_us = intAt(5, "timestamp_us");
  qi::AnyReference ref = anyref[6].content();
  if (ref.kind() != qi::TypeKind_Raw) throw Failure("Could not retrieve buffer");
  result.buffer = (void*)ref.asRaw().first;
  check(ref.asRaw().second == size_t(result.width) * result.height * result.number_of_layers, "buffer size");
  result.cam_id = intAt(7, "cam_id");
  result.fov_left = floatAt(8, "fov_left");
  result.fov_top = floatAt(9, "fov_top");
  result.fov_right = floatAt(10, "fov_right");
  result.fov_bottom = floatAt(11, "fov_bottom");
  return result;
}

std::vector<float> fromAnyValueToFloatVector(qi::AnyValue& value) {
  std::vector<float> result;
  qi::AnyReferenceVector anyrefs = value.asListValuePtr();
  for (size_t i = 0; i < anyrefs.size(); i++) result.push_back(anyrefs[i].content().toFloat());
  return result;
}

std::vector<std::string> fromAnyValueToStringVector(qi::AnyValue& value) {
  std::vector<std::string> result;
  qi::AnyReferenceVector anyrefs = value.asListValuePtr();
  for (size_t i = 0; i < anyrefs.size(); i++) result.push_back(anyrefs[i].content().toString());
  return result;
}

std::vector<std::vector<float>> fromAnyValueToFloatVectorVector(qi::AnyValue& value) {
  std::vector<std::vector<float>> result;
  qi::AnyReferenceVector anyrefs = value.asListValuePtr();
  result.resize(anyrefs.size());
  for (size_t i = 0; i < anyrefs.size(); i++) {
    qi::AnyReferenceVector anyref = anyrefs[i].asListValuePtr();
    result[i].resize(anyref.size());
    for (size_t j = 0; j < anyref.size(); j++) result[i][j] = anyref[j].content().toFloat();
  }
  return result;
}

// naoqi_driver2 src/event/audio.hpp.
class AudioEventRegister {
public:
  void processRemote(int nbOfChannels, int samplesByChannel, qi::AnyValue altimestamp, qi::AnyValue buffer) {
    std::pair<char*, size_t> raw = buffer.asRaw();
    std::lock_guard<std::mutex> lock(mutex);
    channels = nbOfChannels;
    samples = samplesByChannel;
    bytes = raw.second;
    timestampKind = altimestamp.kind();
    ++count;
  }
  std::mutex mutex;
  int channels = 0, samples = 0;
  size_t bytes = 0;
  qi::TypeKind timestampKind = qi::TypeKind_Unknown;
  int count = 0;
};

std::mutex logMutex;
std::vector<qi::LogMessage> logMessages;

void logCallback(const qi::LogMessage& msg) {
  std::lock_guard<std::mutex> lock(logMutex);
  logMessages.push_back(msg);
}

}  // namespace

QI_REGISTER_OBJECT(AudioEventRegister, processRemote)

int main(int argc, char** argv) {
  qi::Application app(argc, argv);
  const std::string url = argc > 1 ? argv[1] : "tcp://127.0.0.1:9559";
  try {
    qi::SessionPtr session = qi::makeSession();
    session->connect(url).value();
    session->listen("tcp://127.0.0.1:0").value();
    ok("connect " + url);

    // Section 3: robot identification.
    qi::AnyObject memory = session->service("ALMemory").value();
    std::string robot = memory.call<std::string>("getData", "RobotConfig/Body/Type");
    std::string baseVersion = memory.call<std::string>("getData", "RobotConfig/Body/BaseVersion");
    std::transform(robot.begin(), robot.end(), robot.begin(), ::tolower);
    check(robot == "nao" || robot == "pepper", "robot type " + robot);
    check(!baseVersion.empty(), "base version");
    ok("ALMemory.getData RobotConfig (" + robot + " " + baseVersion + ")");
    const bool pepper = robot == "pepper";

    qi::AnyObject systemService = session->service("ALSystem").value();
    std::string version = systemService.call<std::string>("systemVersion");
    std::vector<std::string> parts;
    boost::split(parts, version, boost::is_any_of("."));
    check(parts.size() == 4, "version has 4 numbers: " + version);
    ok("ALSystem.systemVersion " + version);

    qi::AnyObject motion = session->service("ALMotion").value();
    std::vector<std::vector<qi::AnyValue>> config =
        motion.call<std::vector<std::vector<qi::AnyValue>>>("getRobotConfig");
    check(config.size() == 2 && config[0].size() == config[1].size(), "getRobotConfig shape");
    bool foundModel = false, foundLegs = false, foundLaser = false;
    for (size_t i = 0; i < config[0].size(); ++i) {
      const std::string name = config[0][i].as<std::string>();
      if (name == "Model Type") { foundModel = true; check(!config[1][i].as<std::string>().empty(), "model type"); }
      if (name == "Number of Legs") { foundLegs = true; check(config[1][i].as<int>() == (pepper ? 0 : 2), "legs"); }
      if (name == "Laser") { foundLaser = true; check(config[1][i].as<bool>() == pepper, "laser"); }
    }
    check(foundModel && foundLegs && foundLaser, "getRobotConfig names");
    ok("ALMotion.getRobotConfig as<std::string>/as<int>/as<bool>");

    std::vector<std::string> sensors = motion.call<std::vector<std::string>>("getSensorNames");
    const bool hasStereo = std::find(sensors.begin(), sensors.end(), "CameraStereo") != sensors.end();
    check(hasStereo == pepper, "CameraStereo on Pepper only");
    ok("ALMotion.getSensorNames");

    // NAOqi 2.9 fallback: ALRobotModel.
    qi::AnyObject robotModel = session->service("ALRobotModel").value();
    check(!robotModel.call<std::string>("getRobotType").empty(), "getRobotType");
    check(robotModel.call<bool>("hasLegs") == !pepper, "hasLegs");
    check(robotModel.call<std::string>("getConfig").find("<ModulePreference") != std::string::npos, "getConfig xml");
    ok("ALRobotModel.getRobotType/hasLegs/getConfig");

    // Section 7: LogManager, as a typed qicore proxy (log.cpp "TEMPORARY CODE").
    qi::LogManagerPtr logger(session->service("LogManager").value());
    qi::AnyObject p_manager = session->service("LogManager").value();
    auto test_obj = p_manager.call<qi::AnyObject>("getListener");
    qi::LogListenerPtr listener = static_cast<qi::LogListenerPtr>(test_obj);
    listener->onLogMessage.connect(logCallback);
    ok("LogManager.getListener as qi::LogListenerPtr");

    // ALBodyTemperature.
    qi::AnyObject bodyTemperature = session->service("ALBodyTemperature").value();
    bodyTemperature.call<void>("setEnableNotifications", true);
    ok("ALBodyTemperature.setEnableNotifications");

    // Diagnostics.
    std::vector<std::string> actuators = motion.call<std::vector<std::string>>("getBodyNames", "JointActuators");
    check(!actuators.empty(), "JointActuators");
    std::vector<std::string> diagnosticKeys;
    for (const auto& joint : actuators) {
      qi::AnyValue limits = motion.call<qi::AnyValue>("getLimits", joint);
      std::vector<std::vector<float>> jointLimits = fromAnyValueToFloatVectorVector(limits);
      check(jointLimits.size() == 1 && jointLimits[0].size() == 4, "limits of " + joint);
      check(jointLimits[0][0] < jointLimits[0][1] && jointLimits[0][2] > 0, "limit values of " + joint);
      diagnosticKeys.push_back("Device/SubDeviceList/" + joint + "/Temperature/Sensor/Value");
      diagnosticKeys.push_back("Device/SubDeviceList/" + joint + "/Hardness/Actuator/Value");
    }
    ok("ALMotion.getLimits for " + std::to_string(actuators.size()) + " joints");
    diagnosticKeys.push_back("BatteryChargeChanged");
    diagnosticKeys.push_back("BatteryPowerPluggedChanged");
    diagnosticKeys.push_back("BatteryFullChargedFlagChanged");
    diagnosticKeys.push_back("Device/SubDeviceList/Battery/Current/Sensor/Value");
    qi::AnyValue diagnostics = memory.call<qi::AnyValue>("getListData", diagnosticKeys);
    std::vector<float> diagnosticValues = fromAnyValueToFloatVector(diagnostics);
    check(diagnosticValues.size() == diagnosticKeys.size(), "diagnostics values");
    check(int(diagnosticValues[diagnosticKeys.size() - 4]) >= 0, "battery charge");
    ok("ALMemory.getListData diagnostics toFloat");

    // Cameras: the driver passes the frame rate as a float.
    qi::AnyObject video = session->service("ALVideoDevice").value();
    float frequency = 10.0f;
    std::string front = video.call<std::string>("subscribeCamera", "front_camera", 0, 1, 11, frequency);
    std::string bottom = video.call<std::string>("subscribeCamera", "bottom_camera", 1, 1, 11, frequency);
    std::vector<std::string> handles = {front, bottom};
    if (pepper) {
      handles.push_back(video.call<std::string>("subscribeCamera", "depth_camera", 2, hasStereo ? 9 : 1, hasStereo ? 17 : 23, frequency));
      if (hasStereo) handles.push_back(video.call<std::string>("subscribeCamera", "stereo_camera", 3, 15, 11, frequency));
      else handles.push_back(video.call<std::string>("subscribeCamera", "infrared_camera", 2, 1, 20, frequency));
    }
    ok("ALVideoDevice.subscribeCamera x" + std::to_string(handles.size()));
    for (const auto& handle : handles) {
      qi::AnyValue image = video.call<qi::AnyValue>("getImageRemote", handle);
      NaoqiImage naoqiImage = fromAnyValueToNaoqiImage(image);
      check(naoqiImage.width > 0 && naoqiImage.height > 0 && naoqiImage.buffer != nullptr, "image of " + handle);
    }
    ok("ALVideoDevice.getImageRemote 12-element structure");

    // Joint states.
    std::vector<std::string> body = motion.call<std::vector<std::string>>("getBodyNames", "Body");
    check(!body.empty(), "Body");
    std::vector<float> torso = motion.async<std::vector<float>>("getPosition", "Torso", 1, true).value();
    check(torso.size() == 6, "getPosition Torso");
    std::vector<double> angles = motion.call<std::vector<double>>("getAngles", "Body", true);
    check(angles.size() == body.size(), "getAngles Body");
    for (const auto& joint : body) {
      double velocity = memory.call<double>("getData", "Motion/Velocity/Sensor/" + joint);
      double torque = memory.call<double>("getData", "Motion/Torque/Sensor/" + joint);
      check(std::isfinite(velocity) && std::isfinite(torque), "velocity/torque of " + joint);
    }
    ok("joint states: getPosition, getAngles, Motion/Velocity|Torque");

    // Odometry and teleop.
    std::vector<float> odom = motion.call<std::vector<float>>("getPosition", "Torso", 1, true);
    std::vector<float> velocity = motion.call<std::vector<float>>("getRobotVelocity");
    check(odom.size() == 6 && velocity.size() == 3, "odometry");
    motion.async<void>("move", 0.1f, 0.0f, 0.0f).value();
    std::this_thread::sleep_for(std::chrono::milliseconds(200));
    std::vector<float> moved = motion.call<std::vector<float>>("getPosition", "Torso", 1, true);
    check(moved[0] > odom[0], "the robot moved forward");
    motion.call<void>("stopMove");
    std::vector<std::string> jointNames = {"HeadYaw"};
    std::vector<float> jointAngles = {0.3f};
    motion.async<void>("setAngles", jointNames, jointAngles, 0.5f).value();
    motion.async<void>("changeAngles", jointNames, jointAngles, 0.5f).value();
    double x = 0.02, y = 0.0, yaw = 0.1;
    motion.async<void>("moveTo", x, y, yaw).value();
    ok("ALMotion.move/setAngles/changeAngles/moveTo");

    // Info converter.
    std::vector<std::string> infoKeys = {"RobotConfig/Head/FullHeadId", "Device/DeviceList/ChestBoard/BodyId",
                                         "RobotConfig/Body/Type", "RobotConfig/Body/BaseVersion",
                                         "RobotConfig/Body/Device/LeftArm/Version", "RobotConfig/Body/Version",
                                         "RobotConfig/Body/SoftwareRequirement"};
    qi::AnyValue infoValues = memory.call<qi::AnyValue>("getListData", infoKeys);
    std::vector<std::string> info = fromAnyValueToStringVector(infoValues);
    check(info.size() == infoKeys.size() && info[2] == (pepper ? "Pepper" : "Nao"), "info values");
    ok("ALMemory.getListData info toString");

    // Memory converters of every type.
    check(memory.call<float>("getData", "Device/SubDeviceList/InertialSensor/AccelerometerZ/Sensor/Value") < 0, "float key");
    check(memory.call<int>("getData", "BatteryChargeChanged") > 0, "int key");
    check(memory.call<bool>("getData", "BatteryPowerPluggedChanged") == false, "bool key");
    qi::AnyValue sniffed = memory.call<qi::AnyValue>("getData", "RobotConfig/Body/Type");
    check(sniffed.kind() == qi::TypeKind_String, "type sniffing of a string key");
    sniffed = memory.call<qi::AnyValue>("getData", "BatteryChargeChanged");
    check(sniffed.kind() == qi::TypeKind_Int, "type sniffing of an int key");
    ok("ALMemory.getData typed and sniffed");

    // ALSonar (< 2.9).
    qi::AnyObject sonar = session->service("ALSonar").value();
    sonar.call<void>("subscribe", "ROS");
    std::vector<std::string> sonarKeys = pepper
        ? std::vector<std::string>{"Device/SubDeviceList/Platform/Front/Sonar/Sensor/Value", "Device/SubDeviceList/Platform/Back/Sonar/Sensor/Value"}
        : std::vector<std::string>{"Device/SubDeviceList/US/Left/Sensor/Value", "Device/SubDeviceList/US/Right/Sensor/Value"};
    qi::AnyValue sonarValues = memory.call<qi::AnyValue>("getListData", sonarKeys);
    check(fromAnyValueToFloatVector(sonarValues).size() == 2, "sonar values");
    sonar.call<void>("unsubscribe", "ROS");
    ok("ALSonar.subscribe/unsubscribe + sonar keys");

    // Audio (section 5).
    qi::AnyObject audio = session->service("ALAudioDevice").value();
    int micConfig = 0;
    if (version.rfind("2.8", 0) == 0 || version.rfind("2.9", 0) == 0) {
      std::map<std::string, std::string> configMap = robotModel.call<std::map<std::string, std::string>>("_getConfigMap");
      micConfig = std::atoi(configMap["RobotConfig/Head/Device/Micro/Version"].c_str());
    } else {
      micConfig = robotModel.call<int>("_getMicrophoneConfig");
    }
    check(micConfig >= 0, "microphone config");
    auto audioRegister = boost::make_shared<AudioEventRegister>();
    unsigned int audioServiceId = session->registerService("ROS-Driver-Audio", audioRegister).value();
    audio.call<void>("setClientPreferences", "ROS-Driver-Audio", 48000, 0, 0);
    audio.call<void>("subscribe", "ROS-Driver-Audio");
    waitFor([&] { std::lock_guard<std::mutex> lock(audioRegister->mutex); return audioRegister->count >= 2; }, "audio buffers");
    {
      std::lock_guard<std::mutex> lock(audioRegister->mutex);
      check(audioRegister->channels == 4 && audioRegister->samples == 8192, "audio channels/samples");
      check(audioRegister->bytes == size_t(2 * 4 * 8192), "audio buffer bytes");
      check(audioRegister->timestampKind == qi::TypeKind_List, "audio timestamp is an ALValue list");
    }
    audio.call<void>("unsubscribe", "ROS-Driver-Audio");
    session->unregisterService(audioServiceId).value();
    ok("ALAudioDevice.processRemote callbacks on ROS-Driver-Audio");

    // Touch events (section 8.2).
    std::vector<std::string> touchKeys = {"RightBumperPressed", "LeftBumperPressed", "HandRightBackTouched",
                                          "HandRightLeftTouched", "HandRightRightTouched", "HandLeftBackTouched",
                                          "HandLeftLeftTouched", "HandLeftRightTouched", "FrontTactilTouched",
                                          "MiddleTactilTouched", "RearTactilTouched"};
    if (pepper) touchKeys.push_back("BackBumperPressed");
    std::mutex touchMutex;
    std::map<std::string, float> touched;
    std::vector<std::pair<qi::AnyObject, qi::SignalLink>> subscriptions;
    for (const auto& key : touchKeys) {
      auto subscriber = memory.call<qi::AnyObject>("subscriber", key);
      qi::SignalLink link = subscriber.connect("signal", [&, key](const qi::AnyValue& v) {
        std::lock_guard<std::mutex> lock(touchMutex);
        touched[key] = v.toFloat();
      }).value();
      subscriptions.emplace_back(std::move(subscriber), link);
    }
    memory.call<void>("raiseEvent", "FrontTactilTouched", 1.0f);
    waitFor([&] { std::lock_guard<std::mutex> lock(touchMutex); return touched.count("FrontTactilTouched") > 0; }, "touch event");
    check(touched["FrontTactilTouched"] > 0.5f, "touch pressed");
    for (auto& subscription : subscriptions) subscription.first.disconnect(subscription.second);
    ok("ALMemory.subscriber signals for " + std::to_string(touchKeys.size()) + " touch keys");

    // Speech (section 9).
    qi::AnyObject tts = session->service("ALTextToSpeech").value();
    tts.async<void>("say", "hello from the smoke test").value();
    waitFor([&] {
      std::lock_guard<std::mutex> lock(logMutex);
      for (const auto& msg : logMessages)
        if (msg.message.find("hello from the smoke test") != std::string::npos) return true;
      return false;
    }, "log message of say");
    {
      std::lock_guard<std::mutex> lock(logMutex);
      const qi::LogMessage& msg = logMessages.back();
      std::vector<std::string> results;
      boost::split(results, msg.source, boost::is_any_of(":"));
      check(results.size() >= 3, "log source has file:function:line");
      check(!msg.category.empty(), "log category");
      check(msg.level == qi::LogLevel_Info, "log level");
      std::cout << "  log: [" << msg.category << "] " << msg.message << " (source " << msg.source
                << ", timestamp " << msg.timestamp.tv_sec << ")" << std::endl;
    }
    ok("LogListener.onLogMessage delivers qi::LogMessage");

    qi::AnyObject dialog = session->service("ALDialog").value();
    dialog.call<void>("setLanguage", "English");
    check(dialog.call<std::string>("getLanguage") == "English", "dialog language");
    std::string topicContent = "topic: ~ros_smoke ()\nlanguage: English\nu:(_[ \"yes\" \"no\" ]) $ros_smoke/result=$1\nu:(_*) $ros_smoke/result=$1\n";
    std::string topic = dialog.call<std::string>("loadTopicContent", topicContent);
    dialog.call<void>("activateTopic", topic);
    dialog.call<void>("subscribe", topic);
    dialog.call<void>("setFocus", topic);
    try {
      session->service("ALSpeechRecognition").value().call<void>("_enableFreeSpeechToText");
    } catch (const std::exception& e) {
      std::cout << "  ALSpeechRecognition._enableFreeSpeechToText failed: " << e.what() << std::endl;
    }
    auto resultSubscriber = memory.call<qi::AnyObject>("subscriber", "ros_smoke/result");
    std::mutex resultMutex;
    std::string result;
    qi::SignalLink resultLink = resultSubscriber.connect("signal", [&](const qi::AnyValue& v) {
      std::lock_guard<std::mutex> lock(resultMutex);
      result = v.toString();
    }).value();
    dialog.call<void>("forceInput", "yes");
    waitFor([&] { std::lock_guard<std::mutex> lock(resultMutex); return !result.empty(); }, "dialog result");
    check(result == "yes", "dialog result value");
    resultSubscriber.disconnect(resultLink);
    dialog.call<void>("deactivateTopic", topic);
    dialog.call<void>("unloadTopic", topic);
    ok("ALDialog listen flow with ALMemory.subscriber result");

    // Shutdown.
    for (const auto& handle : handles) video.call<qi::AnyValue>("unsubscribe", handle);
    session->close();
    std::cout << "ALL OK" << std::endl;
    return 0;
  } catch (const std::exception& e) {
    std::cout << "FAILED: " << e.what() << std::endl;
    return 1;
  }
}
