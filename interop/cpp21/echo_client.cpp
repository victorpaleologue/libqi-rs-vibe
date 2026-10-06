// qi-cpp-echo-client (libqi 2.1): minimal smoke client calling add(1, 2).
//
//   qi-cpp-echo-client --qi-url tcp://host:port [--service TestService]

#include <iostream>
#include <stdexcept>
#include <string>

#include <qi/application.hpp>
#include <qitype/anyobject.hpp>
#include <qimessaging/session.hpp>
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
      if (arg == "--qi-url" && i + 1 < argc)
        url = argv[++i];
      else if (arg == "--service" && i + 1 < argc)
        service = argv[++i];
      else if (arg == "--help" || arg == "-h")
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
