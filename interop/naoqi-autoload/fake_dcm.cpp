// Loads a module library the way NAOqi loads a DCM plugin, against the stub of the NAOqi C++ SDK
// (interop/naoqi-sdk-stub): a broker with a `DCM` module and ALMemory, `_createModule(broker)`,
// then cycles of the loop of the DCM (pre-process, sensor update, post-process), checking the
// commands the plugins send.
//
// Usage: fake-dcm <library> <cycles> <sensor key> [alias name expected to receive commands]
// Prints the aliases created and the last command of each alias, one per line.
#include <alcommon/albroker.h>
#include <alcommon/albrokermanager.h>
#include <alcommon/almodule.h>
#include <alproxies/almemoryproxy.h>
#include <alproxies/dcmproxy.h>
#include <dlfcn.h>
#include <cstdio>
#include <cstdlib>
#include <map>
#include <string>
#include <unistd.h>

typedef int (*CreateModule)(boost::shared_ptr<AL::ALBroker>);

int main(int argc, char** argv) {
  if (argc < 4) {
    std::fprintf(stderr, "usage: %s <library> <cycles> <sensor key> [alias]\n", argv[0]);
    return 2;
  }
  const int cycles = std::atoi(argv[2]);
  const std::string sensor = argv[3];

  boost::shared_ptr<AL::ALBrokerManager> manager(new AL::ALBrokerManager);
  boost::shared_ptr<AL::ALBroker> broker(new AL::ALBroker);
  broker->fBrokerManager = manager;
  broker->fDcm.reset(new AL::DCMProxy);
  broker->fMemory.reset(new AL::ALMemoryProxy);
  broker->fMemory->fData[sensor] = 0.0f;
  boost::shared_ptr<AL::ALModuleCore> dcm(new AL::ALModuleCore("DCM"));
  broker->fModules["DCM"] = dcm;
  // The actuators of the eyes, for the example module.
  for (int side = 0; side < 2; ++side)
    for (int angle = 0; angle < 360; angle += 45) {
      char key[128];
      std::snprintf(key, sizeof key, "Device/SubDeviceList/Face/Led/Blue/%s/%dDeg/Actuator/Value",
                    side ? "Right" : "Left", angle);
      broker->fMemory->fData[key] = 0.0f;
    }

  void* handle = dlopen(argv[1], RTLD_NOW | RTLD_GLOBAL);
  if (!handle) {
    std::fprintf(stderr, "dlopen failed: %s\n", dlerror());
    return 1;
  }
  CreateModule create = reinterpret_cast<CreateModule>(dlsym(handle, "_createModule"));
  if (!create || create(broker) != 0) {
    std::fprintf(stderr, "_createModule failed\n");
    return 1;
  }
  std::printf("modules:");
  for (std::map<std::string, boost::shared_ptr<AL::ALModuleCore> >::iterator it = broker->fModules.begin();
       it != broker->fModules.end(); ++it)
    std::printf(" %s", it->first.c_str());
  std::printf("\n");
  for (size_t i = 0; i < broker->fDcm->fAliases.size(); ++i)
    std::printf("alias %s\n", broker->fDcm->fAliases[i].toString().c_str());

  for (int i = 1; i <= cycles; ++i) {
    dcm->stubPreProcess();
    broker->fDcm->fTime += 10;
    broker->fMemory->fData[sensor] = 0.01f * i;
    dcm->stubPostProcess();
    usleep(10000);  // the period of the DCM of NAO
  }
  std::printf("commands %d\n", static_cast<int>(broker->fDcm->fCommands.size()));
  if (!broker->fDcm->fCommands.empty())
    std::printf("last %s\n", broker->fDcm->fCommands.back().toString().c_str());
  std::fflush(stdout);
  if (argc > 4) {
    for (size_t i = 0; i < broker->fDcm->fCommands.size(); ++i)
      if (std::string(broker->fDcm->fCommands[i][0]) == argv[4])
        return 0;
    std::fprintf(stderr, "no command for the alias %s\n", argv[4]);
    return 1;
  }
  return 0;
}
