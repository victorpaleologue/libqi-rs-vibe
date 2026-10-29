// Stub of the NAOqi C++ SDK, for tests only.
#pragma once
#include <alcommon/albroker.h>
#include <alcommon/almodulecore.h>
#include <boost/shared_ptr.hpp>
#include <string>

namespace AL {
class ALModule : public ALModuleCore {
 public:
  ALModule(boost::shared_ptr<ALBroker> broker, const std::string& name)
      : ALModuleCore(name), fBroker(broker) {}
  boost::shared_ptr<ALBroker> getParentBroker() const { return fBroker; }
  void setModuleDescription(const std::string& description) { fDescription = description; }
  template <class T>
  static boost::shared_ptr<T> createModule(boost::shared_ptr<ALBroker> broker, const std::string& name) {
    boost::shared_ptr<T> module(new T(broker, name));
    broker->fModules[name] = module;
    return module;
  }
  std::string fDescription;

 private:
  boost::shared_ptr<ALBroker> fBroker;
};
}  // namespace AL
