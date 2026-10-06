// qi-cpp-naoqi-probe (libqi 2.1): what a NAOqi 2.1 program does with a robot, against a
// simulated robot (naoqi-sim in its NAOqi 2.1 configuration, or any NAOqi).
//
//   qi-cpp-naoqi-probe --qi-url tcp://robot:9559
//
// Steps, one JSON line each, like qi-cpp-client: ALSystem.systemVersion, ALMemory.getData,
// ALMemory.subscriber (the object-returning pattern of NAOqi 2.x, with its `signal`),
// ALMemory.raiseEvent, ALTextToSpeech.say, ALMotion.getAngles.

#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>

#include <boost/bind.hpp>
#include <boost/function.hpp>
#include <boost/make_shared.hpp>
#include <boost/shared_ptr.hpp>
#include <boost/thread/condition_variable.hpp>
#include <boost/thread/mutex.hpp>

#include <qi/application.hpp>
#include <qi/future.hpp>
#include <qimessaging/session.hpp>
#include <qitype/anyobject.hpp>
#include <qitype/anyvalue.hpp>

#include "json.hpp"
#include "stderr_log.hpp"

namespace
{
  int gTimeoutMs = 5000;

  struct StepFailure : std::runtime_error
  {
    explicit StepFailure(const std::string& what) : std::runtime_error(what) {}
  };

  template <typename T>
  T waitValue(qi::Future<T> f, const std::string& what)
  {
    if (f.wait(gTimeoutMs) == qi::FutureState_Running)
      throw StepFailure(what + ": timeout");
    if (f.hasError())
      throw StepFailure(what + ": " + f.error());
    return f.value();
  }

  void waitVoid(qi::Future<void> f, const std::string& what)
  {
    if (f.wait(gTimeoutMs) == qi::FutureState_Running)
      throw StepFailure(what + ": timeout");
    if (f.hasError())
      throw StepFailure(what + ": " + f.error());
  }

  struct Context
  {
    Context() : passed(0), failed(0) {}
    qi::Session session;
    std::string url;
    qi::AnyObject memory;
    int passed;
    int failed;
  };

  struct Events
  {
    boost::mutex mutex;
    boost::condition_variable cv;
    std::vector<std::string> values;
    void push(qi::AnyValue v)
    {
      boost::mutex::scoped_lock lock(mutex);
      values.push_back(v.signature().toString() + ":" +
                       (v.kind() == qi::TypeKind_Int ? ijson::num(static_cast<long long>(v.toInt()))
                                                     : v.kind() == qi::TypeKind_Float ? ijson::num(v.toDouble())
                                                                                      : v.toString()));
      cv.notify_all();
    }
    bool waitFor(size_t count)
    {
      boost::mutex::scoped_lock lock(mutex);
      const boost::system_time deadline = boost::get_system_time() + boost::posix_time::milliseconds(gTimeoutMs);
      while (values.size() < count)
        if (!cv.timed_wait(lock, deadline))
          return values.size() >= count;
      return true;
    }
  };

  void onEvent(boost::shared_ptr<Events> events, qi::AnyValue v) { events->push(v); }

  std::string stepConnect(Context& ctx)
  {
    waitVoid(ctx.session.connect(ctx.url), "connect");
    return ijson::boolean(ctx.session.isConnected());
  }

  std::string stepSystemVersion(Context& ctx)
  {
    qi::AnyObject system = waitValue<qi::AnyObject>(ctx.session.service("ALSystem"), "service(ALSystem)");
    return ijson::str(waitValue<std::string>(system.async<std::string>("systemVersion"), "systemVersion"));
  }

  std::string stepMemoryGetData(Context& ctx)
  {
    ctx.memory = waitValue<qi::AnyObject>(ctx.session.service("ALMemory"), "service(ALMemory)");
    const qi::AnyValue battery = waitValue<qi::AnyValue>(
      ctx.memory.async<qi::AnyValue>("getData", std::string("Device/SubDeviceList/Battery/Charge/Sensor/Value")),
      "getData");
    if (!battery.isValid())
      throw StepFailure("getData returned an empty value");
    return ijson::object({{"signature", ijson::str(battery.signature().toString())},
                          {"value", ijson::num(battery.toDouble())}});
  }

  std::string stepMemorySubscriber(Context& ctx)
  {
    if (!ctx.memory)
      throw StepFailure("ALMemory not available");
    qi::AnyObject sub =
      waitValue<qi::AnyObject>(ctx.memory.async<qi::AnyObject>("subscriber", std::string("Probe/Key")), "subscriber");
    if (!sub || sub.metaObject().signalId("signal") == -1)
      throw StepFailure("subscriber returned no object with a 'signal' signal");
    boost::shared_ptr<Events> events = boost::make_shared<Events>();
    const qi::SignalLink link = waitValue<qi::SignalLink>(
      sub.connect("signal", boost::function<void(qi::AnyValue)>(boost::bind(&onEvent, events, _1))), "connect(signal)");
    waitVoid(ctx.memory.async<void>("raiseEvent", std::string("Probe/Key"), qi::AnyValue::from<int>(42)), "raiseEvent");
    waitVoid(ctx.memory.async<void>("raiseEvent", std::string("Probe/Key"), qi::AnyValue::from<std::string>("abc")),
             "raiseEvent");
    const bool got = events->waitFor(2);
    waitVoid(sub.disconnect(link), "disconnect(signal)");
    std::vector<std::string> items;
    for (size_t i = 0; i < events->values.size(); ++i)
      items.push_back(ijson::str(events->values[i]));
    if (!got)
      throw StepFailure("received " + ijson::num(static_cast<int>(items.size())) + " events, expected 2");
    const qi::AnyValue data =
      waitValue<qi::AnyValue>(ctx.memory.async<qi::AnyValue>("getData", std::string("Probe/Key")), "getData");
    return ijson::object({{"events", ijson::array(items)}, {"data", ijson::str(data.toString())}});
  }

  std::string stepSay(Context& ctx)
  {
    qi::AnyObject tts = waitValue<qi::AnyObject>(ctx.session.service("ALTextToSpeech"), "service(ALTextToSpeech)");
    waitVoid(tts.async<void>("say", std::string("Hello from NAOqi 2.1")), "say");
    return ijson::boolean(true);
  }

  std::string stepGetAngles(Context& ctx)
  {
    qi::AnyObject motion = waitValue<qi::AnyObject>(ctx.session.service("ALMotion"), "service(ALMotion)");
    std::vector<std::string> names;
    names.push_back("HeadYaw");
    names.push_back("HeadPitch");
    const std::vector<float> angles =
      waitValue<std::vector<float> >(motion.async<std::vector<float> >("getAngles", names, true), "getAngles");
    if (angles.size() != 2)
      throw StepFailure("getAngles returned " + ijson::num(static_cast<int>(angles.size())) + " angles");
    std::vector<std::string> items;
    for (size_t i = 0; i < angles.size(); ++i)
      items.push_back(ijson::num(static_cast<double>(angles[i])));
    return ijson::array(items);
  }

  std::string stepClose(Context& ctx)
  {
    ctx.memory.reset();
    waitVoid(ctx.session.close(), "close");
    return ijson::boolean(true);
  }

  typedef std::string (*StepFn)(Context&);
  struct Step
  {
    const char* name;
    StepFn fn;
  };
  const Step kSteps[] = {
    {"connect", stepConnect},
    {"systemVersion", stepSystemVersion},
    {"memory_getData", stepMemoryGetData},
    {"memory_subscriber", stepMemorySubscriber},
    {"say", stepSay},
    {"getAngles", stepGetAngles},
    {"close", stepClose},
  };
}

int main(int argc, char** argv)
{
  Context ctx;
  ctx.url = "tcp://127.0.0.1:9559";
  try
  {
    qi::Application app(argc, argv);
    interop::routeQiLogsToStderr();
    for (int i = 1; i < argc; ++i)
    {
      const std::string arg = argv[i];
      if (arg == "--qi-url" && i + 1 < argc)
        ctx.url = argv[++i];
      else if (arg == "--timeout-ms" && i + 1 < argc)
        gTimeoutMs = atoi(argv[++i]);
      else if (arg == "--help" || arg == "-h")
      {
        std::cout << "usage: qi-cpp-naoqi-probe --qi-url tcp://robot:9559 [--timeout-ms N]\n";
        return 0;
      }
    }
    for (size_t i = 0; i < sizeof(kSteps) / sizeof(kSteps[0]); ++i)
    {
      std::string result;
      bool ok = false;
      try
      {
        result = kSteps[i].fn(ctx);
        ok = true;
      }
      catch (const std::exception& e)
      {
        result = ijson::object({{"error", ijson::str(e.what())}});
      }
      (ok ? ctx.passed : ctx.failed)++;
      std::cout << ijson::object({{"step", ijson::str(kSteps[i].name)}, {"ok", ijson::boolean(ok)}, {"result", result}})
                << std::endl;
    }
    std::cout << ijson::object({{"step", ijson::str("summary")},
                                {"ok", ijson::boolean(ctx.failed == 0)},
                                {"passed", ijson::num(ctx.passed)},
                                {"failed", ijson::num(ctx.failed)}})
              << std::endl;
    return ctx.failed == 0 ? 0 : 1;
  }
  catch (const std::exception& e)
  {
    std::cerr << "qi-cpp-naoqi-probe: error: " << e.what() << "\n";
    return 1;
  }
}
