// qi-cpp-service (libqi 2.1): the interop "TestService" with the 2014 API.
//
//   qi-cpp-service [--qi-url tcp://sd-host:port] [--qi-listen-url URL]
//                  [--qi-standalone] [--name TestService]
//
// Prints "READY" on stdout once the service is registered, "TICK <n>" whenever
// a callback object emits `tick`, and exits cleanly on SIGINT/SIGTERM.
//
// Compared to the 4.0.5 service (interop/cpp/service.cpp): no optionals (the
// type does not exist in 2.1), no cancellation of sleepMs (the protocol has no
// cancel message), and an ALMemory-like trio (subscriber, raiseEvent, getData)
// replicating the object-returning pattern of NAOqi 2.1's ALMemory.subscriber.

#include <iostream>
#include <map>
#include <stdexcept>
#include <string>
#include <vector>

#include <boost/function.hpp>
#include <boost/make_shared.hpp>
#include <boost/shared_ptr.hpp>
#include <boost/thread/mutex.hpp>

#include <qi/application.hpp>
#include <qi/buffer.hpp>
#include <qi/os.hpp>
#include <qi/future.hpp>
#include <qimessaging/applicationsession.hpp>
#include <qimessaging/session.hpp>
#include <qitype/anyobject.hpp>
#include <qitype/anyvalue.hpp>
#include <qitype/dynamicobjectbuilder.hpp>
#include <qitype/property.hpp>
#include <qitype/signal.hpp>

#include "interop_types.hpp"
#include "stderr_log.hpp"

namespace
{
  struct CounterState
  {
    CounterState() : count(0) {}
    boost::mutex mutex;
    int count;
    qi::Signal<int> changed;
  };

  /// The state of an ALMemory-like subscriber object: `signal(m)` is emitted
  /// with the value of each raiseEvent on its key.
  struct SubscriberState
  {
    std::string key;
    qi::Signal<qi::AnyValue> signal;
  };
  typedef boost::shared_ptr<SubscriberState> SubscriberStatePtr;

  struct ServiceState
  {
    ServiceState() : ticks(0) {}
    qi::Signal<int> fired;
    qi::Signal<> voidSignal;
    qi::Property<int> value;
    qi::Property<std::string> text;

    boost::mutex mutex;
    qi::AnyObject lastCallback;
    int ticks;
    std::map<std::string, qi::AnyValue> memory;
    std::vector<SubscriberStatePtr> subscribers;
  };
  typedef boost::shared_ptr<ServiceState> ServiceStatePtr;

  template <typename T>
  void keepAlive(boost::shared_ptr<T>, qi::GenericObject*)
  {
  }

  int counterIncrement(boost::shared_ptr<CounterState> st)
  {
    int v;
    {
      boost::mutex::scoped_lock lock(st->mutex);
      v = ++st->count;
    }
    st->changed(v);
    return v;
  }

  int counterCurrent(boost::shared_ptr<CounterState> st)
  {
    boost::mutex::scoped_lock lock(st->mutex);
    return st->count;
  }

  qi::AnyObject makeCounter()
  {
    boost::shared_ptr<CounterState> st = boost::make_shared<CounterState>();
    qi::DynamicObjectBuilder ob;
    ob.setThreadingModel(qi::ObjectThreadingModel_MultiThread);
    ob.advertiseMethod("increment", boost::function<int()>(boost::bind(&counterIncrement, st)),
                       std::string("Increments the counter and returns the new value"));
    ob.advertiseMethod("current", boost::function<int()>(boost::bind(&counterCurrent, st)),
                       std::string("Returns the current value"));
    ob.advertiseSignal("changed", &st->changed);
    return ob.object(boost::bind(&keepAlive<CounterState>, st, _1));
  }

  // Synchronous: the 2.1 DynamicObjectBuilder cannot bind methods returning futures.
  int sleepMs(int ms)
  {
    qi::os::msleep(ms < 0 ? 0 : static_cast<unsigned int>(ms));
    return ms;
  }

  int add(int a, int b) { return a + b; }
  std::string concat(const std::string& a, const std::string& b) { return a + b; }
  qi::AnyValue echoDynamic(const qi::AnyValue& v) { return v; }
  std::vector<int> echoList(const std::vector<int>& v) { return v; }
  std::map<std::string, int> echoMap(const std::map<std::string, int>& v) { return v; }
  Point2D echoStruct(const Point2D& p) { return p; }
  qi::Buffer echoBuffer(const qi::Buffer& b) { return b; }
  void fail() { throw std::runtime_error("expected failure"); }
  void fire(ServiceStatePtr state, int v) { state->fired(v); }
  void emitVoid(ServiceStatePtr state) { state->voidSignal(); }
  std::string bigString(int size) { return std::string(static_cast<size_t>(size < 0 ? 0 : size), 'x'); }

  void onTick(ServiceStatePtr state, int v)
  {
    {
      boost::mutex::scoped_lock lock(state->mutex);
      ++state->ticks;
    }
    std::cout << "TICK " << v << std::endl;
  }

  int useCallback(ServiceStatePtr state, qi::AnyObject cb, int n)
  {
    const int result = cb.call<int>("compute", n);
    if (cb.metaObject().signalId("tick") != -1)
    {
      cb.connect("tick", boost::function<void(int)>(boost::bind(&onTick, state, _1))).value();
      boost::mutex::scoped_lock lock(state->mutex);
      state->lastCallback = cb;
    }
    return result;
  }

  // ---- ALMemory-like members ------------------------------------------------

  qi::AnyObject subscriber(ServiceStatePtr state, const std::string& key)
  {
    SubscriberStatePtr st = boost::make_shared<SubscriberState>();
    st->key = key;
    {
      boost::mutex::scoped_lock lock(state->mutex);
      state->subscribers.push_back(st);
    }
    qi::DynamicObjectBuilder ob;
    ob.setThreadingModel(qi::ObjectThreadingModel_MultiThread);
    ob.setDescription("ALMemory-like subscriber of " + key);
    ob.advertiseSignal("signal", &st->signal);
    return ob.object(boost::bind(&keepAlive<SubscriberState>, st, _1));
  }

  void raiseEvent(ServiceStatePtr state, const std::string& key, const qi::AnyValue& value)
  {
    std::vector<SubscriberStatePtr> targets;
    {
      boost::mutex::scoped_lock lock(state->mutex);
      state->memory[key] = value;
      for (size_t i = 0; i < state->subscribers.size(); ++i)
        if (state->subscribers[i]->key == key)
          targets.push_back(state->subscribers[i]);
    }
    for (size_t i = 0; i < targets.size(); ++i)
      targets[i]->signal(value);
  }

  qi::AnyValue getData(ServiceStatePtr state, const std::string& key)
  {
    boost::mutex::scoped_lock lock(state->mutex);
    std::map<std::string, qi::AnyValue>::const_iterator it = state->memory.find(key);
    if (it == state->memory.end())
      throw std::runtime_error("ALMemory::getData: no such key " + key);
    return it->second;
  }

  qi::AnyObject buildService(ServiceStatePtr state)
  {
    qi::DynamicObjectBuilder ob;
    ob.setThreadingModel(qi::ObjectThreadingModel_MultiThread);
    ob.setDescription("libqi interop test service");

    ob.advertiseMethod("add", &add, std::string("Returns a + b"));
    ob.advertiseMethod("concat", &concat, std::string("Returns a followed by b"));
    ob.advertiseMethod("echoDynamic", &echoDynamic, std::string("Returns its dynamic argument unchanged"));
    ob.advertiseMethod("echoList", &echoList, std::string("Returns its list argument unchanged"));
    ob.advertiseMethod("echoMap", &echoMap, std::string("Returns its map argument unchanged"));
    ob.advertiseMethod("echoStruct", &echoStruct, std::string("Returns its Point2D argument unchanged"));
    ob.advertiseMethod("echoBuffer", &echoBuffer, std::string("Returns its raw buffer argument unchanged"));
    ob.advertiseMethod("fail", &fail, std::string("Always fails with the error 'expected failure'"));
    ob.advertiseMethod("sleepMs", &sleepMs, std::string("Waits ms milliseconds then returns ms"));
    ob.advertiseMethod("fire", boost::function<void(int)>(boost::bind(&fire, state, _1)), std::string("Emits the signal fired(v)"));
    ob.advertiseMethod("makeCounter", &makeCounter, std::string("Returns a new counter object (increment, current, changed)"));
    ob.advertiseMethod("useCallback", boost::function<int(qi::AnyObject, int)>(boost::bind(&useCallback, state, _1, _2)),
                       std::string("Calls cb.compute(n), subscribes to cb.tick if present, returns the result"));
    ob.advertiseMethod("emitVoid", boost::function<void()>(boost::bind(&emitVoid, state)),
                       std::string("Emits the parameterless signal voidSignal"));
    ob.advertiseMethod("bigString", &bigString, std::string("Returns a string of `size` 'x' characters"));
    ob.advertiseMethod("subscriber",
                       boost::function<qi::AnyObject(const std::string&)>(boost::bind(&subscriber, state, _1)),
                       std::string("ALMemory-like: returns a subscriber object whose signal(m) carries the values raised on key"));
    ob.advertiseMethod("raiseEvent",
                       boost::function<void(const std::string&, const qi::AnyValue&)>(
                         boost::bind(&raiseEvent, state, _1, _2)),
                       std::string("ALMemory-like: stores the value and emits it to the subscribers of key"));
    ob.advertiseMethod("getData",
                       boost::function<qi::AnyValue(const std::string&)>(boost::bind(&getData, state, _1)),
                       std::string("ALMemory-like: returns the last value raised on key"));

    ob.advertiseSignal("fired", &state->fired);
    ob.advertiseSignal("voidSignal", &state->voidSignal);
    ob.advertiseProperty("value", &state->value);
    ob.advertiseProperty("text", &state->text);

    return ob.object(boost::bind(&keepAlive<ServiceState>, state, _1));
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
        std::cout << "usage: qi-cpp-service [--name NAME] [--qi-url URL] [--qi-listen-url URL] [--qi-standalone]\n";
        return 0;
      }
      else
        std::cerr << "qi-cpp-service: ignoring unknown argument " << arg << "\n";
    }

    app.start();

    ServiceStatePtr state = boost::make_shared<ServiceState>();
    state->value.set(0);
    state->text.set("");
    qi::AnyObject service = buildService(state);

    const unsigned int id = app.session()->registerService(name, service).value();
    std::cerr << "qi-cpp-service: registered '" << name << "' with id " << id;
    const std::vector<qi::Url> endpoints = app.session()->endpoints();
    for (size_t i = 0; i < endpoints.size(); ++i)
      std::cerr << " endpoint " << endpoints[i].str();
    std::cerr << "\n";
    std::cout << "READY" << std::endl;

    app.run();

    try { app.session()->unregisterService(id).wait(1000); } catch (...) {}
    try { app.session()->close().wait(2000); } catch (...) {}
    return 0;
  }
  catch (const std::exception& e)
  {
    std::cerr << "qi-cpp-service: error: " << e.what() << "\n";
    return 1;
  }
}
