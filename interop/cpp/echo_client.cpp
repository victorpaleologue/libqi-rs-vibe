// qi-cpp-echo-client: minimal smoke client.
//
//   qi-cpp-echo-client --qi-url tcp://host:port [--service TestService]
//
// Connects to the service directory, fetches the service and calls
// add(1, 2) once, printing the result ("3") on stdout. Exit code 0 on success.

#include <iostream>
#include <string>

#include <qi/application.hpp>
#include <qi/anyobject.hpp>
#include <qi/session.hpp>
#include "stderr_log.hpp"

int main(int argc, char** argv)
{
  std::string url = "tcp://127.0.0.1:9559";
  std::string service = "TestService";
  try
  {
    qi::Application app(argc, argv);
    interop::routeQiLogsToStderr();
    for (int i = 1; i < argc; ++i)
    {
      const std::string arg = argv[i];
      auto value = [&](const std::string& name, std::string& out) {
        if (arg == name && i + 1 < argc) { out = argv[++i]; return true; }
        if (arg.compare(0, name.size() + 1, name + "=") == 0) { out = arg.substr(name.size() + 1); return true; }
        return false;
      };
      if (value("--qi-url", url) || value("--service", service))
        continue;
      if (arg == "--help" || arg == "-h")
      {
        std::cout << "usage: qi-cpp-echo-client --qi-url URL [--service NAME]\n";
        return 0;
      }
    }

    qi::Session session;
    session.connect(url).value();
    qi::AnyObject obj = session.service(service).value();
    const int result = obj.call<int>("add", 1, 2);
    std::cout << result << std::endl;
    session.close().wait();
    return result == 3 ? 0 : 2;
  }
  catch (const std::exception& e)
  {
    std::cerr << "qi-cpp-echo-client: error: " << e.what() << "\n";
    return 1;
  }
}
