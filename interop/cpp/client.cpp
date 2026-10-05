// qi-cpp-client: reference libqi client exercising the interop "TestService".
//
//   qi-cpp-client --qi-url tcp://sd-host:port [--service TestService]
//                 [--scenarios all|list|step1,step2,...] [--timeout-ms 5000]
//
// Runs a fixed list of scenarios against the service (which may be implemented
// by a foreign implementation), printing one JSON line per step:
//   {"step": "<name>", "ok": true|false, "result": <json>}
// followed by a final {"step":"summary", ...} line. Exit code is 0 only if
// every step passed.

#include <algorithm>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <cstring>
#include <functional>
#include <iostream>
#include <map>
#include <mutex>
#include <set>
#include <string>
#include <thread>
#include <vector>

#include <boost/function.hpp>
#include <boost/optional.hpp>

#include <qi/anyobject.hpp>
#include <qi/anyvalue.hpp>
#include <qi/application.hpp>
#include <qi/buffer.hpp>
#include <qi/future.hpp>
#include <qi/session.hpp>
#include <qi/signal.hpp>
#include <qi/uri.hpp>
#include <qi/type/dynamicobjectbuilder.hpp>

#include "interop_types.hpp"
#include "json.hpp"
#include "stderr_log.hpp"

namespace
{
  int gTimeoutMs = 5000;

  /// Thread-safe collector for signal payloads.
  template <typename T>
  class Collector
  {
  public:
    void push(const T& v)
    {
      std::lock_guard<std::mutex> lock(_m);
      _values.push_back(v);
      _cv.notify_all();
    }
    bool waitFor(size_t count, int timeoutMs)
    {
      std::unique_lock<std::mutex> lock(_m);
      return _cv.wait_for(lock, std::chrono::milliseconds(timeoutMs), [&] { return _values.size() >= count; });
    }
    std::vector<T> values()
    {
      std::lock_guard<std::mutex> lock(_m);
      return _values;
    }

  private:
    std::mutex _m;
    std::condition_variable _cv;
    std::vector<T> _values;
  };

  struct StepFailure : std::runtime_error
  {
    using std::runtime_error::runtime_error;
  };

  template <typename T>
  T waitValue(qi::Future<T> f, const std::string& what)
  {
    if (f.wait(gTimeoutMs) == qi::FutureState_Running)
    {
      f.cancel();
      throw StepFailure(what + ": timeout after " + std::to_string(gTimeoutMs) + "ms");
    }
    if (f.hasError())
      throw StepFailure(what + ": " + f.error());
    if (f.isCanceled())
      throw StepFailure(what + ": canceled");
    return f.value();
  }

  void waitVoid(qi::Future<void> f, const std::string& what)
  {
    if (f.wait(gTimeoutMs) == qi::FutureState_Running)
    {
      f.cancel();
      throw StepFailure(what + ": timeout after " + std::to_string(gTimeoutMs) + "ms");
    }
    if (f.hasError())
      throw StepFailure(what + ": " + f.error());
    if (f.isCanceled())
      throw StepFailure(what + ": canceled");
  }

  template <typename R, typename... Args>
  R call(const qi::AnyObject& obj, const std::string& method, Args&&... args)
  {
    return waitValue<R>(obj.async<R>(method, std::forward<Args>(args)...), method);
  }

  template <typename... Args>
  void callVoid(const qi::AnyObject& obj, const std::string& method, Args&&... args)
  {
    waitVoid(obj.async<void>(method, std::forward<Args>(args)...), method);
  }

  std::string jsonInts(const std::vector<int>& v)
  {
    std::vector<std::string> parts;
    for (int i : v)
      parts.push_back(ijson::num(i));
    return ijson::array(parts);
  }

  struct Context
  {
    qi::Session session;
    std::string url;
    std::string serviceName;
    qi::AnyObject service;
    int passed = 0;
    int failed = 0;
  };

  using StepFn = std::function<std::string(Context&)>; // returns the JSON "result"

  struct Step
  {
    std::string name;
    StepFn fn;
  };

  void runStep(Context& ctx, const Step& step)
  {
    std::string result;
    bool ok = false;
    try
    {
      result = step.fn(ctx);
      ok = true;
    }
    catch (const std::exception& e)
    {
      result = ijson::object({{"error", ijson::str(e.what())}});
    }
    catch (...)
    {
      result = ijson::object({{"error", ijson::str("unknown exception")}});
    }
    (ok ? ctx.passed : ctx.failed)++;
    std::cout << ijson::object({{"step", ijson::str(step.name)}, {"ok", ijson::boolean(ok)}, {"result", result}})
              << std::endl;
  }

  void expect(bool cond, const std::string& message)
  {
    if (!cond)
      throw StepFailure(message);
  }

  // ---------------------------------------------------------------- steps --

  std::string stepConnect(Context& ctx)
  {
    waitVoid(ctx.session.connect(ctx.url).async(), "connect");
    return ijson::object({{"url", ijson::str(ctx.url)}, {"connected", ijson::boolean(ctx.session.isConnected())}});
  }

  std::string stepSdServices(Context& ctx)
  {
    const std::vector<qi::ServiceInfo> infos = waitValue(ctx.session.services().async(), "services");
    std::vector<std::string> items;
    bool found = false;
    for (const auto& si : infos)
    {
      if (si.name() == ctx.serviceName)
        found = true;
      std::vector<std::string> eps;
      // uriEndpoints() keeps relative endpoints such as "qi:ServiceDirectory"
      // intact (qi::Url::str() would render them as "qi://").
      for (const auto& ep : si.uriEndpoints())
        eps.push_back(ijson::str(qi::to_string(ep)));
      items.push_back(ijson::object({{"name", ijson::str(si.name())},
                                     {"serviceId", ijson::num(si.serviceId())},
                                     {"machineId", ijson::str(si.machineId())},
                                     {"processId", ijson::num(si.processId())},
                                     {"sessionId", ijson::str(si.sessionId())},
                                     {"objectUidLen", ijson::num(static_cast<unsigned>(si.objectUid().size()))},
                                     {"endpoints", ijson::array(eps)}}));
    }
    expect(found, "service '" + ctx.serviceName + "' not listed by the service directory");
    return ijson::array(items);
  }

  std::string stepSdMachineId(Context& ctx)
  {
    qi::AnyObject sd = waitValue(ctx.session.service("ServiceDirectory").async(), "service(ServiceDirectory)");
    const std::string machineId = call<std::string>(sd, "machineId");
    expect(!machineId.empty(), "empty machineId");
    return ijson::str(machineId);
  }

  std::string stepGetService(Context& ctx)
  {
    ctx.service = waitValue(ctx.session.service(ctx.serviceName).async(), "service(" + ctx.serviceName + ")");
    expect(static_cast<bool>(ctx.service), "null service object");
    const qi::MetaObject& mo = ctx.service.metaObject();
    std::vector<std::string> methods, signals, properties;
    for (const auto& m : mo.methodMap())
      if (m.first >= 100)
        methods.push_back(ijson::str(m.second.toString()));
    for (const auto& s : mo.signalMap())
      if (s.first >= 100)
        signals.push_back(ijson::str(s.second.toString()));
    for (const auto& p : mo.propertyMap())
      if (p.first >= 100)
        properties.push_back(ijson::str(p.second.toString()));
    return ijson::object({{"methods", ijson::array(methods)},
                          {"signals", ijson::array(signals)},
                          {"properties", ijson::array(properties)}});
  }

  void requireService(Context& ctx)
  {
    if (!ctx.service)
      throw StepFailure("service object not available (get_service failed)");
  }

  std::string stepAdd(Context& ctx)
  {
    requireService(ctx);
    const int r = call<int>(ctx.service, "add", 1, 2);
    expect(r == 3, "add(1,2) returned " + std::to_string(r));
    return ijson::num(r);
  }

  std::string stepConcat(Context& ctx)
  {
    requireService(ctx);
    const std::string r = call<std::string>(ctx.service, "concat", std::string("foo"), std::string("bar"));
    expect(r == "foobar", "concat returned '" + r + "'");
    return ijson::str(r);
  }

  std::string stepEchoDynamicInt(Context& ctx)
  {
    requireService(ctx);
    qi::AnyValue r = call<qi::AnyValue>(ctx.service, "echoDynamic", qi::AnyValue::from<int>(42));
    expect(r.isValid(), "echoDynamic returned an empty value");
    const int v = r.to<int>();
    expect(v == 42, "echoDynamic(42) returned " + std::to_string(v));
    return ijson::object({{"signature", ijson::str(r.signature().toString())}, {"value", ijson::num(v)}});
  }

  std::string stepEchoDynamicString(Context& ctx)
  {
    requireService(ctx);
    qi::AnyValue r = call<qi::AnyValue>(ctx.service, "echoDynamic", qi::AnyValue::from<std::string>("héllo"));
    expect(r.isValid(), "echoDynamic returned an empty value");
    const std::string v = r.to<std::string>();
    expect(v == "héllo", "echoDynamic string returned '" + v + "'");
    return ijson::object({{"signature", ijson::str(r.signature().toString())}, {"value", ijson::str(v)}});
  }

  std::string stepEchoDynamicList(Context& ctx)
  {
    requireService(ctx);
    const std::vector<int> in{1, 2, 3};
    qi::AnyValue r = call<qi::AnyValue>(ctx.service, "echoDynamic", qi::AnyValue::from(in));
    expect(r.isValid(), "echoDynamic returned an empty value");
    const std::vector<int> v = r.to<std::vector<int>>();
    expect(v == in, "echoDynamic list mismatch");
    return ijson::object({{"signature", ijson::str(r.signature().toString())}, {"value", jsonInts(v)}});
  }

  std::string stepEchoDynamicStruct(Context& ctx)
  {
    requireService(ctx);
    qi::AnyValue r = call<qi::AnyValue>(ctx.service, "echoDynamic", qi::AnyValue::from(Point2D{4, 2}));
    expect(r.isValid(), "echoDynamic returned an empty value");
    const Point2D p = r.to<Point2D>();
    expect(p.x == 4 && p.y == 2, "echoDynamic struct mismatch");
    return ijson::object({{"signature", ijson::str(r.signature().toString())},
                          {"x", ijson::num(p.x)},
                          {"y", ijson::num(p.y)}});
  }

  std::string stepEchoDynamicEmpty(Context& ctx)
  {
    requireService(ctx);
    qi::AnyValue r = call<qi::AnyValue>(ctx.service, "echoDynamic", qi::AnyValue());
    // An empty dynamic must come back empty (signature "" on the wire) or void.
    const bool empty = !r.isValid() || r.kind() == qi::TypeKind_Void;
    expect(empty, "echoDynamic(empty) returned a non-empty value with signature " + r.signature().toString());
    return ijson::object({{"signature", ijson::str(r.isValid() ? r.signature().toString() : std::string())}});
  }

  std::string stepEchoList(Context& ctx)
  {
    requireService(ctx);
    const std::vector<int> in{1, -2, 300000};
    const std::vector<int> r = call<std::vector<int>>(ctx.service, "echoList", in);
    expect(r == in, "echoList mismatch");
    return jsonInts(r);
  }

  std::string stepEchoListEmpty(Context& ctx)
  {
    requireService(ctx);
    const std::vector<int> in;
    const std::vector<int> r = call<std::vector<int>>(ctx.service, "echoList", in);
    expect(r.empty(), "echoList([]) not empty");
    return jsonInts(r);
  }

  std::string stepEchoMap(Context& ctx)
  {
    requireService(ctx);
    const std::map<std::string, int> in{{"a", 1}, {"b", 2}, {"héllo", -3}};
    const std::map<std::string, int> r = call<std::map<std::string, int>>(ctx.service, "echoMap", in);
    expect(r == in, "echoMap mismatch");
    std::vector<std::pair<std::string, std::string>> fields;
    for (const auto& kv : r)
      fields.emplace_back(kv.first, ijson::num(kv.second));
    return ijson::object(fields);
  }

  std::string stepEchoStruct(Context& ctx)
  {
    requireService(ctx);
    const Point2D r = call<Point2D>(ctx.service, "echoStruct", Point2D{4, 2});
    expect(r.x == 4 && r.y == 2, "echoStruct mismatch");
    return ijson::object({{"x", ijson::num(r.x)}, {"y", ijson::num(r.y)}});
  }

  std::string stepEchoOptionalNone(Context& ctx)
  {
    requireService(ctx);
    const boost::optional<int> r = call<boost::optional<int>>(ctx.service, "echoOptional", boost::optional<int>());
    expect(!r, "echoOptional(none) returned a value");
    return "null";
  }

  std::string stepEchoOptionalSome(Context& ctx)
  {
    requireService(ctx);
    const boost::optional<int> r = call<boost::optional<int>>(ctx.service, "echoOptional", boost::optional<int>(7));
    expect(r && *r == 7, "echoOptional(7) mismatch");
    return ijson::num(*r);
  }

  std::string stepEchoBuffer(Context& ctx)
  {
    requireService(ctx);
    qi::Buffer in;
    const std::uint8_t bytes[] = {0x2a, 0x00, 0xff, 0x10, 0x80};
    in.write(bytes, sizeof(bytes));
    const qi::Buffer r = call<qi::Buffer>(ctx.service, "echoBuffer", in);
    std::vector<std::uint8_t> out(r.size());
    if (!out.empty())
      r.read(out.data(), 0, out.size());
    expect(out.size() == sizeof(bytes) && std::memcmp(out.data(), bytes, sizeof(bytes)) == 0, "echoBuffer mismatch");
    return ijson::str(ijson::hex(out));
  }

  std::string stepFail(Context& ctx)
  {
    requireService(ctx);
    qi::Future<void> f = ctx.service.async<void>("fail");
    expect(f.wait(gTimeoutMs) != qi::FutureState_Running, "fail(): timeout");
    expect(f.hasError(), "fail() did not fail");
    const std::string err = f.error();
    expect(err.find("expected failure") != std::string::npos, "unexpected error text: " + err);
    return ijson::str(err);
  }

  std::string stepSleepShort(Context& ctx)
  {
    requireService(ctx);
    const int r = call<int>(ctx.service, "sleepMs", 50);
    expect(r == 50, "sleepMs(50) returned " + std::to_string(r));
    return ijson::num(r);
  }

  std::string stepSleepCancel(Context& ctx)
  {
    requireService(ctx);
    qi::Future<int> f = ctx.service.async<int>("sleepMs", 5000);
    std::this_thread::sleep_for(std::chrono::milliseconds(100));
    const auto t0 = std::chrono::steady_clock::now();
    f.cancel();
    const qi::FutureState state = f.wait(gTimeoutMs);
    const auto elapsedMs =
      std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - t0).count();
    expect(state != qi::FutureState_Running, "sleepMs(5000) still running after cancel");
    expect(f.isCanceled(), std::string("sleepMs(5000) was not canceled: ") +
                             (f.hasError() ? f.error() : std::string("finished with value")));
    return ijson::object({{"canceled", ijson::boolean(true)}, {"cancel_latency_ms", ijson::num(static_cast<long long>(elapsedMs))}});
  }

  std::string stepSignalFired(Context& ctx)
  {
    requireService(ctx);
    auto collector = std::make_shared<Collector<int>>();
    const qi::SignalLink link = waitValue(
      ctx.service.connect("fired", boost::function<void(int)>([collector](int v) { collector->push(v); })).async(),
      "connect(fired)");
    expect(qi::isValidSignalLink(link), "invalid signal link");
    callVoid(ctx.service, "fire", 1);
    callVoid(ctx.service, "fire", 2);
    const bool got = collector->waitFor(2, gTimeoutMs);
    waitVoid(ctx.service.disconnect(link).async(), "disconnect(fired)");
    const std::vector<int> values = collector->values();
    expect(got, "received " + std::to_string(values.size()) + " fired events, expected 2");
    expect(values.size() == 2 && values[0] == 1 && values[1] == 2, "unexpected fired payloads " + jsonInts(values));
    return ijson::object({{"link", ijson::num(static_cast<unsigned long long>(link))}, {"events", jsonInts(values)}});
  }

  std::string stepSignalVoid(Context& ctx)
  {
    requireService(ctx);
    auto collector = std::make_shared<Collector<int>>();
    const qi::SignalLink link = waitValue(
      ctx.service.connect("voidSignal", boost::function<void()>([collector]() { collector->push(1); })).async(),
      "connect(voidSignal)");
    callVoid(ctx.service, "emitVoid");
    const bool got = collector->waitFor(1, gTimeoutMs);
    waitVoid(ctx.service.disconnect(link).async(), "disconnect(voidSignal)");
    expect(got, "voidSignal event not received");
    return ijson::object({{"events", ijson::num(static_cast<int>(collector->values().size()))}});
  }

  std::string stepPropertyValue(Context& ctx)
  {
    requireService(ctx);
    auto collector = std::make_shared<Collector<int>>();
    const qi::SignalLink link = waitValue(
      ctx.service.connect("value", boost::function<void(int)>([collector](int v) { collector->push(v); })).async(),
      "connect(value)");
    waitVoid(ctx.service.setProperty("value", 42).async(), "setProperty(value)");
    const int r = waitValue(ctx.service.property<int>("value").async(), "property(value)");
    const bool got = collector->waitFor(1, gTimeoutMs);
    waitVoid(ctx.service.disconnect(link).async(), "disconnect(value)");
    expect(r == 42, "property value read back " + std::to_string(r));
    const std::vector<int> events = collector->values();
    expect(got, "no property change event received for 'value'");
    expect(std::find(events.begin(), events.end(), 42) != events.end(),
           "property change events did not contain 42: " + jsonInts(events));
    return ijson::object({{"value", ijson::num(r)}, {"events", jsonInts(events)}});
  }

  std::string stepPropertyText(Context& ctx)
  {
    requireService(ctx);
    waitVoid(ctx.service.setProperty("text", std::string("héllo")).async(), "setProperty(text)");
    const std::string r = waitValue(ctx.service.property<std::string>("text").async(), "property(text)");
    expect(r == "héllo", "property text read back '" + r + "'");
    return ijson::str(r);
  }

  std::string stepPropertyGeneric(Context& ctx)
  {
    requireService(ctx);
    // Exercise the low-level property(id)/setProperty(id, AnyValue) path.
    const int id = ctx.service.metaObject().propertyId("value");
    expect(id >= 0, "no property 'value' in the meta object");
    waitVoid(ctx.service.setProperty(static_cast<unsigned int>(id), qi::AnyValue::from<int>(7)).async(),
             "setProperty(id)");
    const qi::AnyValue r = waitValue(ctx.service.property(static_cast<unsigned int>(id)).async(), "property(id)");
    expect(r.isValid() && r.to<int>() == 7, "generic property read back mismatch");
    return ijson::object({{"id", ijson::num(id)},
                          {"signature", ijson::str(r.signature().toString())},
                          {"value", ijson::num(r.to<int>())}});
  }

  std::string stepMakeCounter(Context& ctx)
  {
    requireService(ctx);
    qi::AnyObject counter = call<qi::AnyObject>(ctx.service, "makeCounter");
    expect(static_cast<bool>(counter), "makeCounter returned a null object");
    auto collector = std::make_shared<Collector<int>>();
    const qi::SignalLink link = waitValue(
      counter.connect("changed", boost::function<void(int)>([collector](int v) { collector->push(v); })).async(),
      "connect(changed)");
    const int a = call<int>(counter, "increment");
    const int b = call<int>(counter, "increment");
    const int c = call<int>(counter, "current");
    const bool got = collector->waitFor(2, gTimeoutMs);
    waitVoid(counter.disconnect(link).async(), "disconnect(changed)");
    expect(a == 1 && b == 2 && c == 2,
           "counter values " + std::to_string(a) + "," + std::to_string(b) + "," + std::to_string(c));
    const std::vector<int> events = collector->values();
    expect(got && events.size() >= 2 && events[0] == 1 && events[1] == 2, "changed events " + jsonInts(events));
    const std::string uid = [&] {
      std::ostringstream os;
      os << counter.uid();
      return os.str();
    }();
    counter.reset(); // drop the remote object (sends terminate)
    return ijson::object({{"increments", jsonInts({a, b})}, {"current", ijson::num(c)}, {"events", jsonInts(events)},
                          {"uid", ijson::str(uid)}});
  }

  struct CallbackState
  {
    std::atomic<int> computeCalls{0};
    std::atomic<int> lastArg{0};
    qi::Signal<int> tick;
  };

  std::string stepUseCallback(Context& ctx)
  {
    requireService(ctx);
    auto st = boost::make_shared<CallbackState>();
    qi::DynamicObjectBuilder ob;
    ob.setThreadingModel(qi::ObjectThreadingModel_MultiThread);
    ob.advertiseMethod("compute", boost::function<int(int)>([st](int n) {
                         ++st->computeCalls;
                         st->lastArg = n;
                         return n * 2;
                       }),
                       std::string("Returns n * 2"));
    ob.advertiseSignal("tick", &st->tick);
    qi::AnyObject cb = ob.object(st);

    const int r = call<int>(ctx.service, "useCallback", cb, 21);
    expect(r == 42, "useCallback returned " + std::to_string(r));
    expect(st->computeCalls.load() == 1, "compute called " + std::to_string(st->computeCalls.load()) + " times");
    expect(st->lastArg.load() == 21, "compute received " + std::to_string(st->lastArg.load()));
    // Emit tick once after the call returned; the service may have subscribed.
    st->tick(7);
    std::this_thread::sleep_for(std::chrono::milliseconds(200));
    return ijson::object({{"result", ijson::num(r)},
                          {"computeCalls", ijson::num(st->computeCalls.load())},
                          {"tickSubscribers", ijson::boolean(st->tick.hasSubscribers())}});
  }

  std::string stepBigString(Context& ctx)
  {
    requireService(ctx);
    const int size = 200000;
    const std::string r = call<std::string>(ctx.service, "bigString", size);
    expect(static_cast<int>(r.size()) == size, "bigString returned " + std::to_string(r.size()) + " bytes");
    const bool allX = std::all_of(r.begin(), r.end(), [](char c) { return c == 'x'; });
    return ijson::object({{"size", ijson::num(static_cast<int>(r.size()))}, {"all_x", ijson::boolean(allX)}});
  }

  std::string stepClose(Context& ctx)
  {
    ctx.service.reset();
    waitVoid(ctx.session.close().async(), "close");
    return ijson::boolean(!ctx.session.isConnected());
  }

  std::vector<Step> allSteps()
  {
    return {
      {"connect", stepConnect},
      {"sd_services", stepSdServices},
      {"sd_machineId", stepSdMachineId},
      {"get_service", stepGetService},
      {"add", stepAdd},
      {"concat", stepConcat},
      {"echoDynamic_int", stepEchoDynamicInt},
      {"echoDynamic_string", stepEchoDynamicString},
      {"echoDynamic_list", stepEchoDynamicList},
      {"echoDynamic_struct", stepEchoDynamicStruct},
      {"echoDynamic_empty", stepEchoDynamicEmpty},
      {"echoList", stepEchoList},
      {"echoList_empty", stepEchoListEmpty},
      {"echoMap", stepEchoMap},
      {"echoStruct", stepEchoStruct},
      {"echoOptional_none", stepEchoOptionalNone},
      {"echoOptional_some", stepEchoOptionalSome},
      {"echoBuffer", stepEchoBuffer},
      {"fail", stepFail},
      {"sleepMs_short", stepSleepShort},
      {"sleepMs_cancel", stepSleepCancel},
      {"signal_fired", stepSignalFired},
      {"signal_void", stepSignalVoid},
      {"property_value", stepPropertyValue},
      {"property_text", stepPropertyText},
      {"property_generic", stepPropertyGeneric},
      {"makeCounter", stepMakeCounter},
      {"useCallback", stepUseCallback},
      {"bigString", stepBigString},
      {"close", stepClose},
    };
  }

  void usage()
  {
    std::cout << "usage: qi-cpp-client --qi-url tcp://host:port [--service NAME] "
                 "[--scenarios all|list|a,b,c] [--timeout-ms N]\n";
  }
}

int main(int argc, char** argv)
{
  std::string url = "tcp://127.0.0.1:9559";
  std::string serviceName = "TestService";
  std::string scenarios = "all";
  try
  {
    qi::Application app(argc, argv);
    interop::routeQiLogsToStderr();
    for (int i = 1; i < argc; ++i)
    {
      const std::string arg = argv[i];
      auto value = [&](const std::string& name, std::string& out) {
        if (arg == name && i + 1 < argc) { out = argv[++i]; return true; }
        if (arg.compare(0, name.size() + 1, name + "=") == 0) { out = arg.substr(name.size() + 1); return true; }
        return false;
      };
      std::string timeout;
      if (value("--qi-url", url) || value("--service", serviceName) || value("--scenarios", scenarios))
        continue;
      if (value("--timeout-ms", timeout))
      {
        gTimeoutMs = std::stoi(timeout);
        continue;
      }
      if (arg == "--help" || arg == "-h")
      {
        usage();
        return 0;
      }
      std::cerr << "qi-cpp-client: ignoring unknown argument " << arg << "\n";
    }

    const std::vector<Step> steps = allSteps();
    if (scenarios == "list")
    {
      for (const auto& s : steps)
        std::cout << s.name << "\n";
      return 0;
    }

    std::set<std::string> selected;
    if (scenarios != "all")
    {
      size_t start = 0;
      while (start <= scenarios.size())
      {
        const size_t comma = scenarios.find(',', start);
        const std::string item = scenarios.substr(start, comma == std::string::npos ? std::string::npos : comma - start);
        if (!item.empty())
          selected.insert(item);
        if (comma == std::string::npos)
          break;
        start = comma + 1;
      }
      for (const auto& name : selected)
      {
        if (std::none_of(steps.begin(), steps.end(), [&](const Step& s) { return s.name == name; }))
        {
          std::cerr << "qi-cpp-client: unknown scenario '" << name << "'\n";
          return 2;
        }
      }
      // connect / get_service / close are always needed.
      selected.insert("connect");
      selected.insert("get_service");
      selected.insert("close");
    }

    Context ctx;
    ctx.url = url;
    ctx.serviceName = serviceName;
    for (const auto& step : steps)
    {
      if (!selected.empty() && !selected.count(step.name))
        continue;
      runStep(ctx, step);
    }
    const bool allOk = ctx.failed == 0;
    std::cout << ijson::object({{"step", ijson::str("summary")},
                                {"ok", ijson::boolean(allOk)},
                                {"passed", ijson::num(ctx.passed)},
                                {"failed", ijson::num(ctx.failed)}})
              << std::endl;
    return allOk ? 0 : 1;
  }
  catch (const std::exception& e)
  {
    std::cerr << "qi-cpp-client: error: " << e.what() << "\n";
    return 1;
  }
}
