// The libqi entry points of modules written in Rust (feature `module-cpp` of the `qi` crate).
//
// A Rust module serves its members from a private space on the loopback interface (see
// `qi/src/module.rs`). This shim makes them members of a C++ `qi` module, built against the libqi
// SDK of the host:
//
// - libqi 2.3 and later: `qi_module_init` (called by `qi::import`, hence by qilaunch,
//   `Session::loadService` and `Session::callModule`) fills the `qi::ModuleBuilder` with one
//   forwarder per member of the Rust module object, which calls the member through a C++
//   `qi::Session` connected to the private space. Each member has two overloads, with and without a
//   leading `qi::SessionPtr`: libqi passes the session of the host first when a member accepts it,
//   and its URL becomes the space the Rust session connects to.
// - libqi 2.1: the factories of the module are registered when the library is loaded
//   (`QI_REGISTER_OBJECT_FACTORY`), for `Session::loadService(library)` (the `qi-launch` of
//   NAOqi 2.1) to register their objects as services.
//
// Nothing here may throw to libqi or block its network thread. The shim has no namespace-scope
// object with a constructor: the registration of libqi 2.1 runs from a constructor of the Rust
// library, which may run before the C++ ones.

// Boost 1.73 and later warn about the global placeholders of `boost::bind`, which Boost 1.54
// (libqi 2.1) only has.
#define BOOST_BIND_GLOBAL_PLACEHOLDERS
#include <boost/bind.hpp>
#include <boost/make_shared.hpp>
#include <boost/thread/mutex.hpp>
#include <cstddef>
#include <stdexcept>
#include <string>
#include <vector>

#if defined(QI_RS_SHIM_LIBQI21)
#include <qi/log.hpp>
#include <qi/os.hpp>
#include <qimessaging/session.hpp>
#include <qitype/anyobject.hpp>
#include <qitype/dynamicobjectbuilder.hpp>
#include <qitype/objectfactory.hpp>
#else
#include <qi/anymodule.hpp>
#include <qi/log.hpp>
#include <qi/os.hpp>
#include <qi/session.hpp>
#endif

// The log category is declared in each function that logs: `qiLogCategory` is a static.
#define QI_RS_LOG_CATEGORY qiLogCategory("qi.module.rust")

extern "C" {
// The descriptor of a Rust module: `ModuleV1` in qi/src/module/abi.rs.
struct QiRsFactoryV1 {
  const char* name;
  unsigned int flags;
};

struct QiRsModuleV1 {
  unsigned int abi_version;
  const char* name;
  int (*set_option)(const char* key, const char* value);
  int (*start)(char* out, size_t out_len);
  size_t factory_count;
  const QiRsFactoryV1* factories;
};
}

namespace {

const unsigned int supportedAbiVersion = 1;
const unsigned int firstUserMemberId = 100;

// The connection to the private space of the Rust module, created on first use and kept for
// the life of the process: the objects the host registers are proxies through it.
struct Bridge {
  const QiRsModuleV1* descriptor;
  qi::SessionPtr session;
  qi::AnyObject module;
  bool spaceUrlSet;
};

boost::mutex& bridgeMutex() {
  static boost::mutex* mutex = new boost::mutex;
  return *mutex;
}

Bridge*& bridgeInstance() {
  static Bridge* instance = 0;
  return instance;
}

// Connects to the private space of the module, starting it. Throws on failure.
Bridge* bridge(const QiRsModuleV1* descriptor) {
  QI_RS_LOG_CATEGORY;
  boost::mutex::scoped_lock lock(bridgeMutex());
  Bridge*& instance = bridgeInstance();
  if (instance)
    return instance;
  if (descriptor->abi_version != supportedAbiVersion)
    throw std::runtime_error("unsupported ABI version of the Rust module");
  // libqi keeps the loopback endpoints of the services of its own machine only.
  const std::string machineId = qi::os::getMachineId();
  descriptor->set_option("machine-id", machineId.c_str());
  std::vector<char> out(4096, '\0');
  if (descriptor->start(&out[0], out.size()) != 0)
    throw std::runtime_error("the Rust module failed to start: " + std::string(&out[0]));
  const std::string endpoint(&out[0]);
  qi::SessionPtr session = boost::make_shared<qi::Session>();
  session->connect(endpoint).value();
  qi::AnyObject module = session->service(descriptor->name).value();
  if (!module)
    throw std::runtime_error("the Rust module is not in its private space");
  qiLogVerbose() << "Rust module " << descriptor->name << " connected at " << endpoint;
  Bridge* created = new Bridge;
  created->descriptor = descriptor;
  created->session = session;
  created->module = module;
  created->spaceUrlSet = false;
  instance = created;
  return created;
}

#if !defined(QI_RS_SHIM_LIBQI21)

// The URL of the space of a session: the service directory it is connected to, or, for a
// session hosting the space (NAOqi), its own loopback endpoint if it has one.
std::string spaceUrl(const qi::SessionPtr& session) {
  const qi::Url url = session->url();
  const std::vector<qi::Url> endpoints = session->endpoints();
  bool hosting = false;
  for (size_t i = 0; i < endpoints.size(); ++i)
    hosting = hosting || endpoints[i].str() == url.str();
  if (hosting) {
    for (size_t i = 0; i < endpoints.size(); ++i) {
      const qi::Url& endpoint = endpoints[i];
      if (endpoint.protocol() == "tcp" &&
          (endpoint.host() == "127.0.0.1" || endpoint.host() == "localhost"))
        return endpoint.str();
    }
  }
  return url.str();
}

void noteSession(Bridge* bridge, qi::AnyReference argument) {
  QI_RS_LOG_CATEGORY;
  {
    boost::mutex::scoped_lock lock(bridgeMutex());
    if (bridge->spaceUrlSet)
      return;
    bridge->spaceUrlSet = true;
  }
  try {
    // The type interface of `qi::SessionPtr` instantiated in this library is not the one of
    // libqi, and libqi does not convert between them: compare the type identities instead.
    if (!argument.type() || !(argument.type()->info() == qi::typeOf<qi::SessionPtr>()->info()))
      throw std::runtime_error("unexpected type " +
                               (argument.type() ? argument.type()->info().asString() : "none"));
    qi::SessionPtr* pointer = argument.ptr<qi::SessionPtr>(false);
    const qi::SessionPtr session = pointer ? *pointer : qi::SessionPtr();
    if (session) {
      const std::string url = spaceUrl(session);
      bridge->descriptor->set_option("space-url", url.c_str());
      qiLogVerbose() << "Rust module " << bridge->descriptor->name << " uses the space at " << url;
    }
  } catch (const std::exception& e) {
    qiLogVerbose() << "the first argument is not a session: " << e.what();
  }
}

void adaptResult(qi::Future<qi::AnyReference> result, qi::Promise<qi::AnyValue> promise) {
  if (result.hasError())
    promise.setError(result.error());
  else if (result.isCanceled())
    promise.setCanceled();
  else
    promise.setValue(qi::AnyValue(result.value(), false, true));
}

// A member of the module: forwards the call to the Rust module object, asynchronously. libqi
// unwraps the future returned.
qi::AnyReference forward(Bridge* bridge, unsigned int member, bool withSession,
                         const qi::AnyReferenceVector& args) {
  qi::Promise<qi::AnyValue> promise;
  try {
    size_t first = 0;
    if (withSession && !args.empty()) {
      noteSession(bridge, args[0]);
      first = 1;
    }
    qi::GenericFunctionParameters params;
    for (size_t i = first; i < args.size(); ++i)
      params.push_back(args[i]);
    qi::Future<qi::AnyReference> result =
        bridge->module.metaCall(member, params, qi::MetaCallType_Queued);
    result.connect(boost::bind(&adaptResult, _1, promise));
  } catch (const std::exception& e) {
    promise.setError(e.what());
  }
  return qi::AnyReference::from(promise.future()).clone();
}

void advertise(qi::ModuleBuilder* builder, Bridge* bridge, const qi::MetaMethod& method,
               bool withSession) {
  std::string parameters = method.parametersSignature().toString();
  if (withSession)
    parameters = "(" + qi::typeOf<qi::SessionPtr>()->signature().toString() + parameters.substr(1);
  qi::AnyFunction function =
      qi::AnyFunction::fromDynamicFunction(boost::bind(&forward, bridge, method.uid(), withSession, _1));
  // The first argument of the functions of a dynamic object is the object itself.
  function.dropFirstArgument();
  builder->xAdvertiseMethod(method.returnSignature(), method.name(), qi::Signature(parameters),
                            function, method.description());
}

#else  // QI_RS_SHIM_LIBQI21

// A factory of the module: creates the object through the Rust module object. libqi 2.1 calls it
// without argument, and terminates its host if it throws or returns no object.
qi::AnyReference createObject21(const QiRsModuleV1* descriptor, size_t factory,
                                const qi::AnyReferenceVector&) {
  QI_RS_LOG_CATEGORY;
  const char* name = descriptor->factories[factory].name;
  try {
    Bridge* connected = bridge(descriptor);
    qi::AnyObject object = connected->module.call<qi::AnyObject>(name);
    if (!object)
      throw std::runtime_error("no object");
    return qi::AnyReference::from(object).clone();
  } catch (const std::exception& e) {
    qiLogError() << "Rust module " << descriptor->name << ": cannot create " << name << ": "
                 << e.what();
  }
  qi::DynamicObjectBuilder empty;
  empty.setDescription(std::string("unavailable: the Rust module ") + descriptor->name +
                       " failed to create it");
  return qi::AnyReference::from(empty.object()).clone();
}

#endif  // QI_RS_SHIM_LIBQI21

}  // namespace

extern "C" {

// `qi_module_init` (libqi 2.3 and later), called by the Rust library.
void qi_rs_shim_module_init(const QiRsModuleV1* descriptor, void* moduleBuilder) {
  QI_RS_LOG_CATEGORY;
#if !defined(QI_RS_SHIM_LIBQI21)
  try {
    qi::ModuleBuilder* builder = static_cast<qi::ModuleBuilder*>(moduleBuilder);
    if (builder->moduleName() != descriptor->name) {
      qiLogError() << "module name do not match: '" << descriptor->name << "' vs '"
                   << builder->moduleName() << "'";
      return;
    }
    Bridge* connected = bridge(descriptor);
    const qi::MetaObject meta = connected->module.metaObject();
    builder->setDescription(meta.description());
    const qi::MetaObject::MethodMap methods = meta.methodMap();
    for (qi::MetaObject::MethodMap::const_iterator it = methods.begin(); it != methods.end(); ++it) {
      if (it->first < firstUserMemberId)
        continue;
      advertise(builder, connected, it->second, true);
      advertise(builder, connected, it->second, false);
    }
  } catch (const std::exception& e) {
    qiLogError() << "Rust module " << descriptor->name << ": " << e.what();
  }
#else
  (void)descriptor;
  (void)moduleBuilder;
#endif
}

// The registration of the factories (libqi 2.1), called when the library is loaded.
void qi_rs_shim_static_init(const QiRsModuleV1* descriptor) {
  QI_RS_LOG_CATEGORY;
#if defined(QI_RS_SHIM_LIBQI21)
  try {
    for (size_t i = 0; i < descriptor->factory_count; ++i)
      qi::registerObjectFactory(
          descriptor->factories[i].name,
          qi::AnyFunction::fromDynamicFunction(boost::bind(&createObject21, descriptor, i, _1)));
  } catch (const std::exception& e) {
    qiLogError() << "Rust module " << descriptor->name << ": " << e.what();
  }
#else
  (void)descriptor;
#endif
}

}  // extern "C"
