// qi-cpp-service: reference libqi implementation of the interop "TestService".
//
//   qi-cpp-service [--qi-url tcp://sd-host:port] [--qi-listen-url URL]
//                  [--qi-standalone] [--name TestService]
//
// Standard qi::ApplicationSession options are accepted (--qi-url to reach a
// service directory, --qi-listen-url to choose the endpoint, --qi-standalone to
// host the service directory in-process). Prints "READY" on stdout once the
// service is registered, and exits cleanly on SIGINT/SIGTERM.
//
// Exposed members (see README.md for the exact signatures):
//   methods   add, concat, echoDynamic, echoList, echoMap, echoStruct,
//             echoOptional, echoBuffer, fail, sleepMs (cancellable), fire,
//             makeCounter, useCallback, emitVoid, bigString
//   signals   fired(int), voidSignal()
//   properties value(int), text(string)

#include <atomic>
#include <iostream>
#include <map>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

#include <boost/function.hpp>
#include <boost/make_shared.hpp>
#include <boost/optional.hpp>
#include <boost/shared_ptr.hpp>
#include <boost/thread/mutex.hpp>

#include <qi/anyobject.hpp>
#include <qi/anyvalue.hpp>
#include <qi/applicationsession.hpp>
#include <qi/async.hpp>
#include <qi/buffer.hpp>
#include <qi/future.hpp>
#include <qi/property.hpp>
#include <qi/session.hpp>
#include <qi/signal.hpp>
#include <qi/type/dynamicobjectbuilder.hpp>

#include "interop_types.hpp"
#include "stderr_log.hpp"

namespace
{
  struct CounterState
  {
    std::atomic<int> count{0};
    qi::Signal<int> changed;
  };

  struct ServiceState
  {
    qi::Signal<int> fired;
    qi::Signal<> voidSignal;
    qi::Property<int> value;
    qi::Property<std::string> text;

    boost::mutex mutex;
    // Keep the last callback object (and its `tick` subscription) alive so
    // that events emitted by the client after useCallback() returns are still
    // delivered to this process.
    qi::AnyObject lastCallback;
    std::atomic<int> ticks{0};
  };

  qi::AnyObject makeCounter()
  {
    auto st = boost::make_shared<CounterState>();
    qi::DynamicObjectBuilder ob;
    ob.setThreadingModel(qi::ObjectThreadingModel_MultiThread);
    ob.advertiseMethod("increment", boost::function<int()>([st]() {
                         const int v = ++st->count;
                         st->changed(v);
                         return v;
                       }),
                       std::string("Increments the counter and returns the new value"));

    ob.advertiseMethod("current", boost::function<int()>([st]() { return st->count.load(); }),
                       std::string("Returns the current value"));

    ob.advertiseSignal("changed", &st->changed);
    // Keep `st` alive as long as the dynamic object exists.
    return ob.object(st);
  }

  qi::Future<int> sleepMs(int ms)
  {
    qi::Promise<int> promise;
    qi::Future<void> timer = qi::asyncDelay(
      [promise, ms]() mutable {
        if (promise.future().isRunning())
        {
          try { promise.setValue(ms); } catch (const std::exception&) {}
        }
      },
      qi::MilliSeconds(ms < 0 ? 0 : ms));
    promise.setOnCancel([timer](qi::Promise<int>& p) mutable {
      timer.cancel();
      if (p.future().isRunning())
      {
        try { p.setCanceled(); } catch (const std::exception&) {}
      }
    });
    return promise.future();
  }

  qi::AnyObject buildService(const boost::shared_ptr<ServiceState>& state)
  {
    qi::DynamicObjectBuilder ob;
    ob.setThreadingModel(qi::ObjectThreadingModel_MultiThread);
    ob.setDescription("libqi interop test service");

    ob.advertiseMethod("add", boost::function<int(int, int)>([](int a, int b) { return a + b; }),
                       std::string("Returns a + b"));

    ob.advertiseMethod("concat",
                       boost::function<std::string(const std::string&, const std::string&)>(
                         [](const std::string& a, const std::string& b) { return a + b; }),
                       std::string("Returns a followed by b"));

    ob.advertiseMethod("echoDynamic",
                       boost::function<qi::AnyValue(const qi::AnyValue&)>(
                         [](const qi::AnyValue& v) { return v; }),
                       std::string("Returns its dynamic argument unchanged"));

    ob.advertiseMethod("echoList",
                       boost::function<std::vector<int>(const std::vector<int>&)>(
                         [](const std::vector<int>& v) { return v; }),
                       std::string("Returns its list argument unchanged"));

    ob.advertiseMethod("echoMap",
                       boost::function<std::map<std::string, int>(const std::map<std::string, int>&)>(
                         [](const std::map<std::string, int>& v) { return v; }),
                       std::string("Returns its map argument unchanged"));

    ob.advertiseMethod("echoStruct",
                       boost::function<Point2D(const Point2D&)>([](const Point2D& p) { return p; }),
                       std::string("Returns its Point2D argument unchanged"));

    ob.advertiseMethod("echoOptional",
                       boost::function<boost::optional<int>(const boost::optional<int>&)>(
                         [](const boost::optional<int>& o) { return o; }),
                       std::string("Returns its optional argument unchanged"));

    ob.advertiseMethod("echoBuffer",
                       boost::function<qi::Buffer(const qi::Buffer&)>([](const qi::Buffer& b) { return b; }),
                       std::string("Returns its raw buffer argument unchanged"));

    ob.advertiseMethod("fail", boost::function<void()>([]() {
                         throw std::runtime_error("expected failure");
                       }),
                       std::string("Always fails with the error 'expected failure'"));

    ob.advertiseMethod("sleepMs", boost::function<qi::Future<int>(int)>(&sleepMs),
                       std::string("Waits ms milliseconds then returns ms; cancellable"));

    ob.advertiseMethod("fire", boost::function<void(int)>([state](int v) {
                         state->fired(v);
                       }),
                       std::string("Emits the signal fired(v)"));

    ob.advertiseMethod("makeCounter", boost::function<qi::AnyObject()>(&makeCounter),
                       std::string("Returns a new counter object (increment, current, changed)"));

    ob.advertiseMethod("useCallback",
                       boost::function<int(qi::AnyObject, int)>([state](qi::AnyObject cb, int n) {
                         const int result = cb.call<int>("compute", n);
                         if (cb.metaObject().signalId("tick") != -1)
                         {
                           auto weak = boost::weak_ptr<ServiceState>(state);
                           cb.connect("tick", boost::function<void(int)>([weak](int v) {
                                        if (auto s = weak.lock())
                                          ++s->ticks;
                                        std::cout << "TICK " << v << std::endl;
                                      }))
                             .value();
                           boost::mutex::scoped_lock lock(state->mutex);
                           state->lastCallback = cb;
                         }
                         return result;
                       }),
                       std::string("Calls cb.compute(n), subscribes to cb.tick if present, returns the result"));

    ob.advertiseMethod("emitVoid", boost::function<void()>([state]() { state->voidSignal(); }),
                       std::string("Emits the parameterless signal voidSignal"));

    ob.advertiseMethod("bigString", boost::function<std::string(int)>([](int size) {
                         return std::string(static_cast<size_t>(size < 0 ? 0 : size), 'x');
                       }),
                       std::string("Returns a string of `size` 'x' characters"));

    ob.advertiseSignal("fired", &state->fired);
    ob.advertiseSignal("voidSignal", &state->voidSignal);
    ob.advertiseProperty("value", &state->value);
    ob.advertiseProperty("text", &state->text);

    return ob.object(state);
  }
}

int main(int argc, char** argv)
{
  try
  {
    qi::ApplicationSession app(argc, argv);
    interop::routeQiLogsToStderr();

    std::string name = "TestService";
    const std::vector<std::string>& args = qi::Application::arguments();
    for (size_t i = 1; i < args.size(); ++i)
    {
      const std::string& arg = args[i];
      if (arg == "--name" && i + 1 < args.size())
        name = args[++i];
      else if (arg.compare(0, 7, "--name=") == 0)
        name = arg.substr(7);
      else if (arg == "--help" || arg == "-h")
      {
        std::cout << "usage: qi-cpp-service [--name NAME] [qi options]\n" << app.helpText() << "\n";
        return 0;
      }
      else
        std::cerr << "qi-cpp-service: ignoring unknown argument " << arg << "\n";
    }

    app.startSession();

    auto state = boost::make_shared<ServiceState>();
    state->value.set(0);
    state->text.set("");
    qi::AnyObject service = buildService(state);

    const unsigned int id = app.session()->registerService(name, service).value();
    std::cerr << "qi-cpp-service: registered '" << name << "' with id " << id;
    for (const auto& ep : app.session()->endpoints())
      std::cerr << " endpoint " << ep.str();
    std::cerr << "\n";
    std::cout << "READY" << std::endl;

    app.run(); // returns on SIGINT/SIGTERM

    // Explicit teardown so the service is unregistered before exit.
    try { app.session()->unregisterService(id).wait(qi::MilliSeconds(1000)); } catch (...) {}
    try { app.session()->close().wait(qi::MilliSeconds(2000)); } catch (...) {}
    return 0;
  }
  catch (const std::exception& e)
  {
    std::cerr << "qi-cpp-service: error: " << e.what() << "\n";
    return 1;
  }
}
