// qi-cpp-client (libqi 2.1): scenario runner against the interop "TestService".
//
//   qi-cpp-client --qi-url tcp://sd-host:port [--service TestService]
//                 [--scenarios all|list|step1,step2,...] [--timeout-ms 5000]
//
// Prints one JSON line per step, {"step": ..., "ok": ..., "result": ...}, then
// a final summary line; exit code 0 only if every step passed. The scenarios
// are those of interop/cpp/client.cpp that the 2.1 API supports (no optionals,
// no remote cancellation), plus the ALMemory-like `memory_subscriber` step.

#include <algorithm>
#include <cstdint>
#include <cstring>
#include <iostream>
#include <map>
#include <set>
#include <sstream>
#include <string>
#include <vector>

#include <boost/bind.hpp>
#include <boost/function.hpp>
#include <boost/make_shared.hpp>
#include <boost/shared_ptr.hpp>
#include <boost/thread/condition_variable.hpp>
#include <boost/thread/mutex.hpp>

#include <qi/application.hpp>
#include <qi/buffer.hpp>
#include <qi/future.hpp>
#include <qi/os.hpp>
#include <qimessaging/session.hpp>
#include <qitype/anyobject.hpp>
#include <qitype/anyvalue.hpp>
#include <qitype/dynamicobjectbuilder.hpp>
#include <qitype/signal.hpp>

#include "interop_types.hpp"
#include "json.hpp"
#include "stderr_log.hpp"

namespace
{
  int gTimeoutMs = 5000;

  template <typename T>
  class Collector
  {
  public:
    void push(const T& v)
    {
      boost::mutex::scoped_lock lock(_m);
      _values.push_back(v);
      _cv.notify_all();
    }
    bool waitFor(size_t count, int timeoutMs)
    {
      boost::mutex::scoped_lock lock(_m);
      const boost::system_time deadline = boost::get_system_time() + boost::posix_time::milliseconds(timeoutMs);
      while (_values.size() < count)
        if (!_cv.timed_wait(lock, deadline))
          return _values.size() >= count;
      return true;
    }
    std::vector<T> values()
    {
      boost::mutex::scoped_lock lock(_m);
      return _values;
    }

  private:
    boost::mutex _m;
    boost::condition_variable _cv;
    std::vector<T> _values;
  };

  struct StepFailure : std::runtime_error
  {
    explicit StepFailure(const std::string& what) : std::runtime_error(what) {}
  };

  template <typename T>
  T waitValue(qi::Future<T> f, const std::string& what)
  {
    if (f.wait(gTimeoutMs) == qi::FutureState_Running)
      throw StepFailure(what + ": timeout after " + ijson::num(gTimeoutMs) + "ms");
    if (f.hasError())
      throw StepFailure(what + ": " + f.error());
    return f.value();
  }

  void waitVoid(qi::Future<void> f, const std::string& what)
  {
    if (f.wait(gTimeoutMs) == qi::FutureState_Running)
      throw StepFailure(what + ": timeout after " + ijson::num(gTimeoutMs) + "ms");
    if (f.hasError())
      throw StepFailure(what + ": " + f.error());
  }

  std::string jsonInts(const std::vector<int>& v)
  {
    std::vector<std::string> parts;
    for (size_t i = 0; i < v.size(); ++i)
      parts.push_back(ijson::num(v[i]));
    return ijson::array(parts);
  }

  struct Context
  {
    Context() : passed(0), failed(0) {}
    qi::Session session;
    std::string url;
    std::string serviceName;
    qi::AnyObject service;
    int passed;
    int failed;
  };

  typedef std::string (*StepFn)(Context&);

  struct Step
  {
    const char* name;
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

  void requireService(Context& ctx)
  {
    if (!ctx.service)
      throw StepFailure("service object not available (get_service failed)");
  }

  template <typename T>
  void collect(boost::shared_ptr<Collector<T> > c, T v) { c->push(v); }
  void collectVoid(boost::shared_ptr<Collector<int> > c) { c->push(1); }

  // ---------------------------------------------------------------- steps --

  std::string stepConnect(Context& ctx)
  {
    waitVoid(ctx.session.connect(ctx.url), "connect");
    return ijson::object({{"url", ijson::str(ctx.url)}, {"connected", ijson::boolean(ctx.session.isConnected())}});
  }

  std::string stepSdServices(Context& ctx)
  {
    const std::vector<qi::ServiceInfo> infos = waitValue<std::vector<qi::ServiceInfo> >(ctx.session.services(), "services");
    std::vector<std::string> items;
    bool found = false;
    for (size_t i = 0; i < infos.size(); ++i)
    {
      const qi::ServiceInfo& si = infos[i];
      if (si.name() == ctx.serviceName)
        found = true;
      std::vector<std::string> eps;
      for (size_t j = 0; j < si.endpoints().size(); ++j)
        eps.push_back(ijson::str(si.endpoints()[j].str()));
      items.push_back(ijson::object({{"name", ijson::str(si.name())},
                                     {"serviceId", ijson::num(si.serviceId())},
                                     {"machineId", ijson::str(si.machineId())},
                                     {"processId", ijson::num(si.processId())},
                                     {"sessionId", ijson::str(si.sessionId())},
                                     {"endpoints", ijson::array(eps)}}));
    }
    expect(found, "service '" + ctx.serviceName + "' not listed by the service directory");
    return ijson::array(items);
  }

  std::string stepSdMachineId(Context& ctx)
  {
    qi::AnyObject sd = waitValue<qi::AnyObject>(ctx.session.service("ServiceDirectory"), "service(ServiceDirectory)");
    const std::string machineId = waitValue<std::string>(sd.async<std::string>("machineId"), "machineId");
    expect(!machineId.empty(), "empty machineId");
    return ijson::str(machineId);
  }

  std::string stepGetService(Context& ctx)
  {
    ctx.service = waitValue<qi::AnyObject>(ctx.session.service(ctx.serviceName), "service(" + ctx.serviceName + ")");
    expect(static_cast<bool>(ctx.service), "null service object");
    const qi::MetaObject& mo = ctx.service.metaObject();
    std::vector<std::string> methods, signals, properties;
    const qi::MetaObject::MethodMap mm = mo.methodMap();
    for (qi::MetaObject::MethodMap::const_iterator it = mm.begin(); it != mm.end(); ++it)
      if (it->first >= 100)
        methods.push_back(ijson::str(it->second.toString()));
    const qi::MetaObject::SignalMap sm = mo.signalMap();
    for (qi::MetaObject::SignalMap::const_iterator it = sm.begin(); it != sm.end(); ++it)
      if (it->first >= 100)
        signals.push_back(ijson::str(it->second.toString()));
    const qi::MetaObject::PropertyMap pm = mo.propertyMap();
    for (qi::MetaObject::PropertyMap::const_iterator it = pm.begin(); it != pm.end(); ++it)
      if (it->first >= 100)
        properties.push_back(ijson::str(it->second.toString()));
    return ijson::object({{"methods", ijson::array(methods)},
                          {"signals", ijson::array(signals)},
                          {"properties", ijson::array(properties)}});
  }

  std::string stepAdd(Context& ctx)
  {
    requireService(ctx);
    const int r = waitValue<int>(ctx.service.async<int>("add", 1, 2), "add");
    expect(r == 3, "add(1,2) returned " + ijson::num(r));
    return ijson::num(r);
  }

  std::string stepConcat(Context& ctx)
  {
    requireService(ctx);
    const std::string r =
      waitValue<std::string>(ctx.service.async<std::string>("concat", std::string("foo"), std::string("bar")), "concat");
    expect(r == "foobar", "concat returned '" + r + "'");
    return ijson::str(r);
  }

  std::string stepEchoDynamicInt(Context& ctx)
  {
    requireService(ctx);
    qi::AnyValue r = waitValue<qi::AnyValue>(ctx.service.async<qi::AnyValue>("echoDynamic", qi::AnyValue::from<int>(42)),
                                             "echoDynamic");
    expect(r.isValid(), "echoDynamic returned an empty value");
    const int v = r.to<int>();
    expect(v == 42, "echoDynamic(42) returned " + ijson::num(v));
    return ijson::object({{"signature", ijson::str(r.signature().toString())}, {"value", ijson::num(v)}});
  }

  std::string stepEchoDynamicString(Context& ctx)
  {
    requireService(ctx);
    qi::AnyValue r = waitValue<qi::AnyValue>(
      ctx.service.async<qi::AnyValue>("echoDynamic", qi::AnyValue::from<std::string>("h\xc3\xa9llo")), "echoDynamic");
    expect(r.isValid(), "echoDynamic returned an empty value");
    const std::string v = r.to<std::string>();
    expect(v == "h\xc3\xa9llo", "echoDynamic string returned '" + v + "'");
    return ijson::object({{"signature", ijson::str(r.signature().toString())}, {"value", ijson::str(v)}});
  }

  std::string stepEchoDynamicList(Context& ctx)
  {
    requireService(ctx);
    std::vector<int> in;
    in.push_back(1);
    in.push_back(2);
    in.push_back(3);
    qi::AnyValue r =
      waitValue<qi::AnyValue>(ctx.service.async<qi::AnyValue>("echoDynamic", qi::AnyValue::from(in)), "echoDynamic");
    expect(r.isValid(), "echoDynamic returned an empty value");
    const std::vector<int> v = r.to<std::vector<int> >();
    expect(v == in, "echoDynamic list mismatch");
    return ijson::object({{"signature", ijson::str(r.signature().toString())}, {"value", jsonInts(v)}});
  }

  std::string stepEchoDynamicStruct(Context& ctx)
  {
    requireService(ctx);
    Point2D in;
    in.x = 4;
    in.y = 2;
    qi::AnyValue r =
      waitValue<qi::AnyValue>(ctx.service.async<qi::AnyValue>("echoDynamic", qi::AnyValue::from(in)), "echoDynamic");
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
    qi::AnyValue r = waitValue<qi::AnyValue>(ctx.service.async<qi::AnyValue>("echoDynamic", qi::AnyValue()), "echoDynamic");
    const bool empty = !r.isValid() || r.kind() == qi::TypeKind_Void;
    expect(empty, "echoDynamic(empty) returned a non-empty value with signature " + r.signature().toString());
    return ijson::object({{"signature", ijson::str(r.isValid() ? r.signature().toString() : std::string())}});
  }

  std::string stepEchoList(Context& ctx)
  {
    requireService(ctx);
    std::vector<int> in;
    in.push_back(1);
    in.push_back(-2);
    in.push_back(300000);
    const std::vector<int> r = waitValue<std::vector<int> >(ctx.service.async<std::vector<int> >("echoList", in), "echoList");
    expect(r == in, "echoList mismatch");
    return jsonInts(r);
  }

  std::string stepEchoListEmpty(Context& ctx)
  {
    requireService(ctx);
    const std::vector<int> in;
    const std::vector<int> r = waitValue<std::vector<int> >(ctx.service.async<std::vector<int> >("echoList", in), "echoList");
    expect(r.empty(), "echoList([]) not empty");
    return jsonInts(r);
  }

  std::string stepEchoMap(Context& ctx)
  {
    requireService(ctx);
    std::map<std::string, int> in;
    in["a"] = 1;
    in["b"] = 2;
    in["h\xc3\xa9llo"] = -3;
    typedef std::map<std::string, int> Map;
    const Map r = waitValue<Map>(ctx.service.async<Map>("echoMap", in), "echoMap");
    expect(r == in, "echoMap mismatch");
    std::vector<std::pair<std::string, std::string> > fields;
    for (Map::const_iterator it = r.begin(); it != r.end(); ++it)
      fields.push_back(std::make_pair(it->first, ijson::num(it->second)));
    return ijson::object(fields);
  }

  std::string stepEchoStruct(Context& ctx)
  {
    requireService(ctx);
    Point2D in;
    in.x = 4;
    in.y = 2;
    const Point2D r = waitValue<Point2D>(ctx.service.async<Point2D>("echoStruct", in), "echoStruct");
    expect(r.x == 4 && r.y == 2, "echoStruct mismatch");
    return ijson::object({{"x", ijson::num(r.x)}, {"y", ijson::num(r.y)}});
  }

  std::string stepEchoBuffer(Context& ctx)
  {
    requireService(ctx);
    qi::Buffer in;
    const std::uint8_t bytes[] = {0x2a, 0x00, 0xff, 0x10, 0x80};
    in.write(bytes, sizeof(bytes));
    const qi::Buffer r = waitValue<qi::Buffer>(ctx.service.async<qi::Buffer>("echoBuffer", in), "echoBuffer");
    std::vector<std::uint8_t> out(r.size());
    if (!out.empty())
      r.read(&out[0], 0, out.size());
    expect(out.size() == sizeof(bytes) && std::memcmp(&out[0], bytes, sizeof(bytes)) == 0, "echoBuffer mismatch");
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
    const int r = waitValue<int>(ctx.service.async<int>("sleepMs", 50), "sleepMs");
    expect(r == 50, "sleepMs(50) returned " + ijson::num(r));
    return ijson::num(r);
  }

  std::string stepSignalFired(Context& ctx)
  {
    requireService(ctx);
    boost::shared_ptr<Collector<int> > collector = boost::make_shared<Collector<int> >();
    const qi::SignalLink link = waitValue<qi::SignalLink>(
      ctx.service.connect("fired", boost::function<void(int)>(boost::bind(&collect<int>, collector, _1))), "connect(fired)");
    expect(link != qi::SignalBase::invalidSignalLink, "invalid signal link");
    waitVoid(ctx.service.async<void>("fire", 1), "fire");
    waitVoid(ctx.service.async<void>("fire", 2), "fire");
    const bool got = collector->waitFor(2, gTimeoutMs);
    waitVoid(ctx.service.disconnect(link), "disconnect(fired)");
    const std::vector<int> values = collector->values();
    expect(got, "received " + ijson::num(static_cast<int>(values.size())) + " fired events, expected 2");
    expect(values.size() == 2 && values[0] == 1 && values[1] == 2, "unexpected fired payloads " + jsonInts(values));
    return ijson::object({{"link", ijson::num(static_cast<unsigned long long>(link))}, {"events", jsonInts(values)}});
  }

  std::string stepSignalVoid(Context& ctx)
  {
    requireService(ctx);
    boost::shared_ptr<Collector<int> > collector = boost::make_shared<Collector<int> >();
    const qi::SignalLink link = waitValue<qi::SignalLink>(
      ctx.service.connect("voidSignal", boost::function<void()>(boost::bind(&collectVoid, collector))),
      "connect(voidSignal)");
    waitVoid(ctx.service.async<void>("emitVoid"), "emitVoid");
    const bool got = collector->waitFor(1, gTimeoutMs);
    waitVoid(ctx.service.disconnect(link), "disconnect(voidSignal)");
    expect(got, "voidSignal event not received");
    return ijson::object({{"events", ijson::num(static_cast<int>(collector->values().size()))}});
  }

  std::string stepPropertyValue(Context& ctx)
  {
    requireService(ctx);
    // The 2.1 client API cannot subscribe to the change signal of a property
    // ("No such signal" for its identifier): only set and get are exercised.
    waitVoid(ctx.service.setProperty("value", 42), "setProperty(value)");
    const int r = waitValue<int>(ctx.service.property<int>("value"), "property(value)");
    expect(r == 42, "property value read back " + ijson::num(r));
    return ijson::object({{"value", ijson::num(r)}});
  }

  std::string stepPropertyText(Context& ctx)
  {
    requireService(ctx);
    waitVoid(ctx.service.setProperty("text", std::string("h\xc3\xa9llo")), "setProperty(text)");
    const std::string r = waitValue<std::string>(ctx.service.property<std::string>("text"), "property(text)");
    expect(r == "h\xc3\xa9llo", "property text read back '" + r + "'");
    return ijson::str(r);
  }

  std::string stepPropertyGeneric(Context& ctx)
  {
    requireService(ctx);
    const int id = ctx.service.metaObject().propertyId("value");
    expect(id >= 0, "no property 'value' in the meta object");
    waitVoid(ctx.service.setProperty(static_cast<unsigned int>(id), qi::AnyValue::from<int>(7)), "setProperty(id)");
    const qi::AnyValue r = waitValue<qi::AnyValue>(ctx.service.property(static_cast<unsigned int>(id)), "property(id)");
    expect(r.isValid() && r.to<int>() == 7, "generic property read back mismatch");
    return ijson::object({{"id", ijson::num(id)},
                          {"signature", ijson::str(r.signature().toString())},
                          {"value", ijson::num(r.to<int>())}});
  }

  std::string stepMakeCounter(Context& ctx)
  {
    requireService(ctx);
    qi::AnyObject counter = waitValue<qi::AnyObject>(ctx.service.async<qi::AnyObject>("makeCounter"), "makeCounter");
    expect(static_cast<bool>(counter), "makeCounter returned a null object");
    boost::shared_ptr<Collector<int> > collector = boost::make_shared<Collector<int> >();
    const qi::SignalLink link = waitValue<qi::SignalLink>(
      counter.connect("changed", boost::function<void(int)>(boost::bind(&collect<int>, collector, _1))), "connect(changed)");
    const int a = waitValue<int>(counter.async<int>("increment"), "increment");
    const int b = waitValue<int>(counter.async<int>("increment"), "increment");
    const int c = waitValue<int>(counter.async<int>("current"), "current");
    const bool got = collector->waitFor(2, gTimeoutMs);
    waitVoid(counter.disconnect(link), "disconnect(changed)");
    expect(a == 1 && b == 2 && c == 2, "counter values " + ijson::num(a) + "," + ijson::num(b) + "," + ijson::num(c));
    const std::vector<int> events = collector->values();
    expect(got && events.size() >= 2 && events[0] == 1 && events[1] == 2, "changed events " + jsonInts(events));
    std::vector<int> increments;
    increments.push_back(a);
    increments.push_back(b);
    counter.reset();
    return ijson::object({{"increments", jsonInts(increments)}, {"current", ijson::num(c)}, {"events", jsonInts(events)}});
  }

  struct CallbackState
  {
    CallbackState() : computeCalls(0), lastArg(0) {}
    boost::mutex mutex;
    int computeCalls;
    int lastArg;
    qi::Signal<int> tick;
  };

  int compute(boost::shared_ptr<CallbackState> st, int n)
  {
    boost::mutex::scoped_lock lock(st->mutex);
    ++st->computeCalls;
    st->lastArg = n;
    return n * 2;
  }

  void keepCallback(boost::shared_ptr<CallbackState>, qi::GenericObject*) {}

  std::string stepUseCallback(Context& ctx)
  {
    requireService(ctx);
    boost::shared_ptr<CallbackState> st = boost::make_shared<CallbackState>();
    qi::DynamicObjectBuilder ob;
    ob.setThreadingModel(qi::ObjectThreadingModel_MultiThread);
    ob.advertiseMethod("compute", boost::function<int(int)>(boost::bind(&compute, st, _1)), std::string("Returns n * 2"));
    ob.advertiseSignal("tick", &st->tick);
    qi::AnyObject cb = ob.object(boost::bind(&keepCallback, st, _1));

    const int r = waitValue<int>(ctx.service.async<int>("useCallback", cb, 21), "useCallback");
    expect(r == 42, "useCallback returned " + ijson::num(r));
    int calls, lastArg;
    {
      boost::mutex::scoped_lock lock(st->mutex);
      calls = st->computeCalls;
      lastArg = st->lastArg;
    }
    expect(calls == 1, "compute called " + ijson::num(calls) + " times");
    expect(lastArg == 21, "compute received " + ijson::num(lastArg));
    st->tick(7);
    qi::os::msleep(200);
    return ijson::object({{"result", ijson::num(r)},
                          {"computeCalls", ijson::num(calls)},
                          {"tickSubscribers", ijson::boolean(st->tick.hasSubscribers())}});
  }

  void collectAny(boost::shared_ptr<Collector<std::string> > c, qi::AnyValue v)
  {
    c->push(v.signature().toString() + ":" + (v.kind() == qi::TypeKind_Int ? ijson::num(static_cast<long long>(v.toInt())) : v.toString()));
  }

  /// ALMemory.subscriber pattern: the service returns an object, the client
  /// subscribes to its `signal(m)` and the service raises values on the key.
  std::string stepMemorySubscriber(Context& ctx)
  {
    requireService(ctx);
    qi::AnyObject sub =
      waitValue<qi::AnyObject>(ctx.service.async<qi::AnyObject>("subscriber", std::string("Test/Key")), "subscriber");
    expect(static_cast<bool>(sub), "subscriber returned a null object");
    expect(sub.metaObject().signalId("signal") != -1, "subscriber object has no 'signal' signal");
    boost::shared_ptr<Collector<std::string> > collector = boost::make_shared<Collector<std::string> >();
    const qi::SignalLink link = waitValue<qi::SignalLink>(
      sub.connect("signal", boost::function<void(qi::AnyValue)>(boost::bind(&collectAny, collector, _1))),
      "connect(signal)");
    waitVoid(ctx.service.async<void>("raiseEvent", std::string("Test/Key"), qi::AnyValue::from<int>(42)), "raiseEvent");
    waitVoid(ctx.service.async<void>("raiseEvent", std::string("Test/Key"), qi::AnyValue::from<std::string>("abc")),
             "raiseEvent");
    const bool got = collector->waitFor(2, gTimeoutMs);
    waitVoid(sub.disconnect(link), "disconnect(signal)");
    const std::vector<std::string> values = collector->values();
    expect(got, "received " + ijson::num(static_cast<int>(values.size())) + " signal events, expected 2");
    expect(values[0] == "i:42" && values[1] == "s:abc", "unexpected signal payloads " + values[0] + " " + values[1]);
    const qi::AnyValue data =
      waitValue<qi::AnyValue>(ctx.service.async<qi::AnyValue>("getData", std::string("Test/Key")), "getData");
    expect(data.isValid() && data.toString() == "abc", "getData mismatch");
    std::vector<std::string> items;
    for (size_t i = 0; i < values.size(); ++i)
      items.push_back(ijson::str(values[i]));
    sub.reset();
    return ijson::object({{"events", ijson::array(items)}, {"data", ijson::str(data.toString())}});
  }

  std::string stepBigString(Context& ctx)
  {
    requireService(ctx);
    const int size = 200000;
    const std::string r = waitValue<std::string>(ctx.service.async<std::string>("bigString", size), "bigString");
    expect(static_cast<int>(r.size()) == size, "bigString returned " + ijson::num(static_cast<int>(r.size())) + " bytes");
    const bool allX = r.find_first_not_of('x') == std::string::npos;
    return ijson::object({{"size", ijson::num(static_cast<int>(r.size()))}, {"all_x", ijson::boolean(allX)}});
  }

  std::string stepClose(Context& ctx)
  {
    ctx.service.reset();
    waitVoid(ctx.session.close(), "close");
    return ijson::boolean(!ctx.session.isConnected());
  }

  const Step kSteps[] = {
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
    {"echoBuffer", stepEchoBuffer},
    {"fail", stepFail},
    {"sleepMs_short", stepSleepShort},
    {"signal_fired", stepSignalFired},
    {"signal_void", stepSignalVoid},
    {"property_value", stepPropertyValue},
    {"property_text", stepPropertyText},
    {"property_generic", stepPropertyGeneric},
    {"makeCounter", stepMakeCounter},
    {"useCallback", stepUseCallback},
    {"memory_subscriber", stepMemorySubscriber},
    {"bigString", stepBigString},
    {"close", stepClose},
  };

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
      if (arg == "--qi-url" && i + 1 < argc)
        url = argv[++i];
      else if (arg == "--service" && i + 1 < argc)
        serviceName = argv[++i];
      else if (arg == "--scenarios" && i + 1 < argc)
        scenarios = argv[++i];
      else if (arg == "--timeout-ms" && i + 1 < argc)
        gTimeoutMs = atoi(argv[++i]);
      else if (arg == "--help" || arg == "-h")
      {
        usage();
        return 0;
      }
      else
        std::cerr << "qi-cpp-client: ignoring unknown argument " << arg << "\n";
    }

    const size_t stepCount = sizeof(kSteps) / sizeof(kSteps[0]);
    if (scenarios == "list")
    {
      for (size_t i = 0; i < stepCount; ++i)
        std::cout << kSteps[i].name << "\n";
      return 0;
    }

    std::set<std::string> selected;
    if (scenarios != "all")
    {
      std::stringstream ss(scenarios);
      std::string item;
      while (std::getline(ss, item, ','))
        if (!item.empty())
          selected.insert(item);
      for (std::set<std::string>::const_iterator it = selected.begin(); it != selected.end(); ++it)
      {
        bool known = false;
        for (size_t i = 0; i < stepCount; ++i)
          if (*it == kSteps[i].name)
            known = true;
        if (!known)
        {
          std::cerr << "qi-cpp-client: unknown scenario '" << *it << "'\n";
          return 2;
        }
      }
      selected.insert("connect");
      selected.insert("get_service");
      selected.insert("close");
    }

    Context ctx;
    ctx.url = url;
    ctx.serviceName = serviceName;
    for (size_t i = 0; i < stepCount; ++i)
    {
      if (!selected.empty() && !selected.count(kSteps[i].name))
        continue;
      runStep(ctx, kSteps[i]);
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
