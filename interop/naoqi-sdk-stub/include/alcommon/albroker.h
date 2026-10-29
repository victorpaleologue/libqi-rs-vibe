// Stub of the NAOqi C++ SDK, for tests only.
#pragma once
#include <alcommon/alproxy.h>
#include <alerror/alerror.h>
#include <boost/shared_ptr.hpp>
#include <boost/weak_ptr.hpp>
#include <map>
#include <string>

namespace AL {
class ALBrokerManager;
class DCMProxy;
class ALMemoryProxy;
class ALBroker {
 public:
  boost::shared_ptr<ALProxy> getProxy(const std::string& name);
  boost::shared_ptr<DCMProxy> getDcmProxy();
  boost::shared_ptr<ALMemoryProxy> getMemoryProxy();
  boost::weak_ptr<ALBrokerManager> fBrokerManager;
  // Test state: the modules of the process, the DCM and ALMemory.
  std::map<std::string, boost::shared_ptr<ALModuleCore> > fModules;
  boost::shared_ptr<DCMProxy> fDcm;
  boost::shared_ptr<ALMemoryProxy> fMemory;
};
}  // namespace AL
