// The NAOqi side of the DCM plugins of Rust modules (feature `module-naoqi` of the `qi` crate),
// built against the C++ SDK of NAOqi (libalcommon, libalproxies, libalvalue).
//
// `_createModule` (Rust) calls `qi_rs_naoqi_create` with the broker NAOqi gives, for each plugin.
// It creates an `ALModule` named after the plugin, the way the DCM plugins written in C++ do (as
// in the `fastgetsetdcm` example of the SDK, mc_naoqi_dcm or rUNSWift's libagent): it takes the
// proxies of the DCM and of ALMemory, lets the Rust plugin get its memory pointers and create its
// aliases, then connects the plugin to the pre-process and post-process callbacks of the DCM.
//
// Nothing here may throw to NAOqi: the errors of the SDK are reported to Rust as messages.

#include <alcommon/albroker.h>
#include <alcommon/albrokermanager.h>
#include <alcommon/almodule.h>
#include <alproxies/almemoryproxy.h>
#include <alproxies/dcmproxy.h>
#include <alvalue/alvalue.h>

#include <boost/bind.hpp>
#include <boost/shared_ptr.hpp>
#include <boost/thread/locks.hpp>
#include <boost/thread/mutex.hpp>
#include <boost/typeof/typeof.hpp>
#include <cstddef>
#include <cstring>
#include <exception>
#include <string>
#include <vector>

extern "C" {
// `ApiV1` and `PluginV1` in qi/src/module/naoqi.rs.
struct QiRsNaoqiApiV1 {
  void* bridge;
  void* (*memory_data_ptr)(void* bridge, const char* key, char* error, size_t error_len);
  int (*dcm_time)(void* bridge, int offset, int* time, char* error, size_t error_len);
  int (*create_alias)(void* bridge, const char* name, const char* const* keys, size_t count,
                      char* error, size_t error_len);
  int (*set_alias)(void* bridge, int alias, int update, int time, const float* values,
                   size_t count, char* error, size_t error_len);
};

struct QiRsNaoqiPluginV1 {
  void* plugin;
  int (*init)(void* plugin, const QiRsNaoqiApiV1* api, char* error, size_t error_len);
  void (*pre_process)(void* plugin);
  void (*post_process)(void* plugin);
  void (*destroy)(void* plugin);
};
}

namespace {

void writeError(const std::string& message, char* out, size_t len) {
  if (!out || len == 0)
    return;
  const size_t count = message.size() < len - 1 ? message.size() : len - 1;
  std::memcpy(out, message.data(), count);
  out[count] = '\0';
}

// The update types of `DCM.setAlias`, in the order of `Update` in naoqi.rs.
const char* updateType(int update) {
  switch (update) {
    case 0: return "Merge";
    case 1: return "ClearAll";
    case 2: return "ClearAfter";
    case 3: return "ClearBefore";
    default: return 0;
  }
}

// The plugin to give to the next `RustDcmPlugin` constructed: `ALModule::createModule` only
// forwards the broker and the name to the constructor. Used under the loader lock of NAOqi.
QiRsNaoqiPluginV1*& pendingPlugin() {
  static QiRsNaoqiPluginV1* plugin = 0;
  return plugin;
}

std::string& pendingError() {
  static std::string* error = new std::string;
  return *error;
}

class RustDcmPlugin : public AL::ALModule {
 public:
  RustDcmPlugin(boost::shared_ptr<AL::ALBroker> broker, const std::string& name)
      : AL::ALModule(broker, name), fPlugin(*pendingPlugin()), fConnected(false), fDestroying(false) {
    setModuleDescription("A DCM plugin written in Rust");
    fApi.bridge = this;
    fApi.memory_data_ptr = &RustDcmPlugin::memoryDataPtr;
    fApi.dcm_time = &RustDcmPlugin::dcmTime;
    fApi.create_alias = &RustDcmPlugin::createAlias;
    fApi.set_alias = &RustDcmPlugin::setAlias;
    pendingError().clear();
    try {
      fDcm = getParentBroker()->getDcmProxy();
      fMemory = getParentBroker()->getMemoryProxy();
    } catch (const std::exception& e) {
      pendingError() = std::string("cannot get the proxies of DCM and ALMemory: ") + e.what();
      return;
    }
    char error[512] = {0};
    if (fPlugin.init(fPlugin.plugin, &fApi, error, sizeof error) != 0) {
      pendingError() = error[0] ? std::string(error) : std::string("the plugin failed to start");
      return;
    }
    try {
      // The DCM is a module of this process: its callbacks run in its loop.
      boost::shared_ptr<AL::ALProxy> dcm = getParentBroker()->getProxy("DCM");
      fPreProcess = dcm->getModule()->atPreProcess(boost::bind(&RustDcmPlugin::onPreProcess, this));
      fPostProcess = dcm->getModule()->atPostProcess(boost::bind(&RustDcmPlugin::onPostProcess, this));
      fConnected = true;
    } catch (const std::exception& e) {
      pendingError() = std::string("cannot connect to the loop of the DCM: ") + e.what();
    }
  }

  virtual ~RustDcmPlugin() {
    if (fConnected) {
      fPreProcess.disconnect();
      fPostProcess.disconnect();
    }
    {
      // A disconnection does not wait for a callback running on the thread of the DCM: wait for
      // it here, and keep the later ones out.
      boost::mutex::scoped_lock lock(fCallbackMutex);
      fDestroying = true;
    }
    fPlugin.destroy(fPlugin.plugin);
  }

 private:
  // The callbacks run on the thread of the DCM, which they never block: a cycle is skipped while
  // the module is being destroyed.
  void onPreProcess() {
    boost::unique_lock<boost::mutex> lock(fCallbackMutex, boost::try_to_lock);
    if (lock.owns_lock() && !fDestroying)
      fPlugin.pre_process(fPlugin.plugin);
  }
  void onPostProcess() {
    boost::unique_lock<boost::mutex> lock(fCallbackMutex, boost::try_to_lock);
    if (lock.owns_lock() && !fDestroying)
      fPlugin.post_process(fPlugin.plugin);
  }

  static RustDcmPlugin* self(void* bridge) { return static_cast<RustDcmPlugin*>(bridge); }

  static void* memoryDataPtr(void* bridge, const char* key, char* error, size_t errorLen) {
    try {
      void* data = self(bridge)->fMemory->getDataPtr(key);
      if (!data)
        writeError(std::string("no data for ") + key, error, errorLen);
      return data;
    } catch (const std::exception& e) {
      writeError(e.what(), error, errorLen);
    } catch (...) {
      writeError("unknown error", error, errorLen);
    }
    return 0;
  }

  static int dcmTime(void* bridge, int offset, int* time, char* error, size_t errorLen) {
    try {
      *time = self(bridge)->fDcm->getTime(offset);
      return 0;
    } catch (const std::exception& e) {
      writeError(e.what(), error, errorLen);
    } catch (...) {
      writeError("unknown error", error, errorLen);
    }
    return -1;
  }

  static int createAlias(void* bridge, const char* name, const char* const* keys, size_t count,
                         char* error, size_t errorLen) {
    RustDcmPlugin* plugin = self(bridge);
    try {
      AL::ALValue alias;
      alias.arraySetSize(2);
      alias[0] = std::string(name);
      alias[1].arraySetSize(static_cast<int>(count));
      for (size_t i = 0; i < count; ++i)
        alias[1][static_cast<int>(i)] = std::string(keys[i]);
      plugin->fDcm->createAlias(alias);

      // The command of the alias, filled by `setAlias`.
      AL::ALValue command;
      command.arraySetSize(6);
      command[0] = std::string(name);
      command[1] = std::string("ClearAll");
      command[2] = std::string("time-separate");
      command[3] = 0;  // the importance level, which must be 0
      command[4].arraySetSize(1);
      command[5].arraySetSize(static_cast<int>(count));
      for (size_t i = 0; i < count; ++i)
        command[5][static_cast<int>(i)].arraySetSize(1);
      plugin->fCommands.push_back(command);
      plugin->fCommandSizes.push_back(count);
      return static_cast<int>(plugin->fCommands.size() - 1);
    } catch (const std::exception& e) {
      writeError(e.what(), error, errorLen);
    } catch (...) {
      writeError("unknown error", error, errorLen);
    }
    return -1;
  }

  static int setAlias(void* bridge, int alias, int update, int time, const float* values,
                      size_t count, char* error, size_t errorLen) {
    RustDcmPlugin* plugin = self(bridge);
    if (alias < 0 || static_cast<size_t>(alias) >= plugin->fCommands.size() ||
        plugin->fCommandSizes[alias] != count) {
      writeError("invalid alias or number of values", error, errorLen);
      return -1;
    }
    const char* type = updateType(update);
    if (!type) {
      writeError("invalid update type", error, errorLen);
      return -1;
    }
    try {
      AL::ALValue& command = plugin->fCommands[alias];
      command[1] = std::string(type);
      command[4][0] = time;
      for (size_t i = 0; i < count; ++i)
        command[5][static_cast<int>(i)][0] = values[i];
      plugin->fDcm->setAlias(command);
      return 0;
    } catch (const std::exception& e) {
      writeError(e.what(), error, errorLen);
    } catch (...) {
      writeError("unknown error", error, errorLen);
    }
    return -1;
  }

  QiRsNaoqiPluginV1 fPlugin;
  QiRsNaoqiApiV1 fApi;
  boost::shared_ptr<AL::DCMProxy> fDcm;
  boost::shared_ptr<AL::ALMemoryProxy> fMemory;
  std::vector<AL::ALValue> fCommands;
  std::vector<size_t> fCommandSizes;
  bool fConnected;
  boost::mutex fCallbackMutex;
  bool fDestroying;
  // The types the SDK gives to the module behind a proxy and to the connections of the
  // callbacks of the DCM (`ProcessSignalConnection`), as the expressions above have them.
  typedef BOOST_TYPEOF(((AL::ALProxy*)0)->getModule()) ModulePointer;
  typedef BOOST_TYPEOF((*(ModulePointer*)0)->atPreProcess(boost::function<void()>())) Connection;
  Connection fPreProcess;
  Connection fPostProcess;
};

}  // namespace

extern "C" int qi_rs_naoqi_create(void* broker, const char* name, const QiRsNaoqiPluginV1* plugin,
                                  char* error, size_t errorLen) {
  QiRsNaoqiPluginV1 owned = *plugin;
  try {
    boost::shared_ptr<AL::ALBroker> parent = *static_cast<boost::shared_ptr<AL::ALBroker>*>(broker);
    // As the modules NAOqi loads do: share the broker manager of NAOqi.
    AL::ALBrokerManager::setInstance(parent->fBrokerManager.lock());
    AL::ALBrokerManager::getInstance()->addBroker(parent);
    pendingPlugin() = &owned;
    pendingError() = "the module was not constructed";
    AL::ALModule::createModule<RustDcmPlugin>(parent, name);
    pendingPlugin() = 0;
    if (!pendingError().empty()) {
      // The module stays, without callbacks, and destroys the plugin with itself.
      writeError(pendingError(), error, errorLen);
      return -1;
    }
    return 0;
  } catch (const std::exception& e) {
    writeError(e.what(), error, errorLen);
  } catch (...) {
    writeError("unknown error", error, errorLen);
  }
  pendingPlugin() = 0;
  return -1;
}
