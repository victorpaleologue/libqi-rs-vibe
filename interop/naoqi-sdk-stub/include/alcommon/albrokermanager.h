// Stub of the NAOqi C++ SDK, for tests only.
#pragma once
#include <boost/shared_ptr.hpp>
#include <vector>

namespace AL {
class ALBroker;
class ALBrokerManager {
 public:
  static void setInstance(boost::shared_ptr<ALBrokerManager> instance);
  static boost::shared_ptr<ALBrokerManager> getInstance();
  void addBroker(boost::shared_ptr<ALBroker> broker) { fBrokers.push_back(broker); }
  std::vector<boost::shared_ptr<ALBroker> > fBrokers;
};
}  // namespace AL
