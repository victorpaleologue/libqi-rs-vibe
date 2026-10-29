// Stub of the NAOqi C++ SDK, for tests only: the implementation of the declarations of include/.
#include <alcommon/albroker.h>
#include <alcommon/albrokermanager.h>
#include <alproxies/almemoryproxy.h>
#include <alproxies/dcmproxy.h>
#include <sstream>

namespace AL {
std::string ALValue::toString() const {
  std::ostringstream out;
  switch (fType) {
    case TypeInvalid: out << "invalid"; break;
    case TypeInt: out << fInt; break;
    case TypeFloat: out << fFloat; break;
    case TypeString: out << '"' << fString << '"'; break;
    case TypeArray:
      out << '[';
      for (size_t i = 0; i < fArray.size(); ++i) out << (i ? ", " : "") << fArray[i].toString();
      out << ']';
  }
  return out.str();
}

ALValue DCMProxy::createAlias(const ALValue& alias) {
  if (alias.getSize() != 2) throw ALERROR("DCM", "createAlias", "bad alias");
  fAliases.push_back(alias);
  return alias;
}

void DCMProxy::setAlias(const ALValue& command) {
  if (command.getSize() != 6) throw ALERROR("DCM", "setAlias", "bad command");
  bool known = false;
  for (size_t i = 0; i < fAliases.size(); ++i)
    known = known || std::string(fAliases[i][0]) == std::string(command[0]);
  if (!known) throw ALERROR("DCM", "setAlias", "unknown alias " + std::string(command[0]));
  fCommands.push_back(command);
}

void* ALMemoryProxy::getDataPtr(const std::string& key) {
  std::map<std::string, float>::iterator it = fData.find(key);
  if (it == fData.end()) throw ALERROR("ALMemory", "getDataPtr", "no key " + key);
  return &it->second;
}

static boost::shared_ptr<ALBrokerManager>& brokerManager() {
  static boost::shared_ptr<ALBrokerManager> instance;
  return instance;
}
void ALBrokerManager::setInstance(boost::shared_ptr<ALBrokerManager> instance) { brokerManager() = instance; }
boost::shared_ptr<ALBrokerManager> ALBrokerManager::getInstance() { return brokerManager(); }

boost::shared_ptr<ALProxy> ALBroker::getProxy(const std::string& name) {
  std::map<std::string, boost::shared_ptr<ALModuleCore> >::iterator it = fModules.find(name);
  if (it == fModules.end()) throw ALERROR("ALBroker", "getProxy", "no module " + name);
  return boost::shared_ptr<ALProxy>(new ALProxy(it->second));
}
boost::shared_ptr<DCMProxy> ALBroker::getDcmProxy() { return fDcm; }
boost::shared_ptr<ALMemoryProxy> ALBroker::getMemoryProxy() { return fMemory; }
}  // namespace AL
